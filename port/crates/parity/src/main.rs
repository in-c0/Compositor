//! `parity`: renders the corpus with the port and checks it against the Mac app's references.

mod affected;
mod cases;
mod compare;
mod corpus_gen;
mod report;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use compare::{CaseResult, Status, Tolerances};
use rayon::prelude::*;
use report::RunResults;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(about = "Pixel parity between the Compositor port and the Mac app")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate the corpus from scratch.
    GenCorpus {
        #[arg(long, default_value = "parity/corpus")]
        out: PathBuf,
    },
    /// Render cases with the port and compare them with the references.
    Run {
        #[arg(long, default_value = "parity/corpus")]
        corpus: PathBuf,
        /// Reference PNGs from the Mac harness, laid out as `<case id>.png`.
        #[arg(long)]
        refs: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "parity/tolerances.toml")]
        tolerances: PathBuf,
        /// `windows` or `mac`.
        #[arg(long)]
        platform: String,
        /// Case id globs; every case when omitted.
        #[arg(long = "case")]
        cases: Vec<String>,
        /// A previous `results.json` from `main`, for the regression check.
        #[arg(long)]
        baseline: Option<PathBuf>,
    },
    /// Write PARITY.md from one `results.json` per platform.
    Report {
        #[arg(long = "results", required = true)]
        results: Vec<PathBuf>,
        #[arg(long, default_value = "parity/features.toml")]
        features: PathBuf,
        #[arg(long, default_value = "parity/tolerances.toml")]
        tolerances: PathBuf,
        #[arg(long, default_value = "PARITY.md")]
        out: PathBuf,
        #[arg(long, default_value = "local")]
        commit: String,
    },
    /// Print the case globs a change since `base` can affect, one per line (`*` for all).
    Affected {
        #[arg(long)]
        base: String,
        #[arg(long, default_value = "parity/features.toml")]
        features: PathBuf,
        #[arg(long, default_value = "parity/corpus")]
        corpus: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::GenCorpus { out } => corpus_gen::generate(&out),
        Command::Run { corpus, refs, out, tolerances, platform, cases, baseline } => {
            run(&corpus, &refs, &out, &tolerances, &platform, &cases, baseline.as_deref())
        }
        Command::Report { results, features, tolerances, out, commit } => {
            let runs = results.iter().map(|p| RunResults::load(p)).collect::<Result<Vec<_>>>()?;
            let features = report::FeatureList::load(&features)?;
            let tolerances = Tolerances::load(&tolerances)?;
            std::fs::write(out, report::parity_markdown(&features, &runs, &tolerances.overrides, &commit))?;
            Ok(())
        }
        Command::Affected { base, features, corpus } => {
            for line in affected::affected(&base, &report::FeatureList::load(&features)?, &corpus)? {
                println!("{line}");
            }
            Ok(())
        }
    }
}

fn run(
    corpus: &Path,
    refs: &Path,
    out: &Path,
    tolerances: &Path,
    platform: &str,
    patterns: &[String],
    baseline: Option<&Path>,
) -> Result<()> {
    let tolerances = Tolerances::load(tolerances)?;
    let cases = cases::select(cases::discover(corpus)?, patterns)?;
    if cases.is_empty() {
        bail!("no cases match");
    }
    let renderer = engine::Renderer::new().context("starting the GPU")?;
    let adapter = renderer.adapter_name();
    eprintln!("rendering {} cases on {adapter}", cases.len());
    std::fs::create_dir_all(out.join("renders"))?;
    std::fs::create_dir_all(out.join("heatmaps"))?;
    let mut results: Vec<CaseResult> = cases.par_iter().map(|case| run_case(&renderer, case, refs, out, &tolerances)).collect();
    let references = std::fs::read(refs.join("harness-info.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null);
    let baseline = match baseline {
        Some(path) if path.exists() => Some(RunResults::load(path)?),
        _ => None,
    };
    // A push re-renders only the cases it can affect. The rest keep their result from `main`,
    // so results.json, and the PARITY.md built from it, still cover the whole corpus.
    if let Some(base) = &baseline {
        let ran: std::collections::HashSet<String> = results.iter().map(|r| r.id.clone()).collect();
        let exists: std::collections::HashSet<String> = cases::discover(corpus)?.into_iter().map(|c| c.id).collect();
        results.extend(base.cases.iter().filter(|c| !ran.contains(&c.id) && exists.contains(&c.id)).cloned());
        results.sort_by(|a, b| a.id.cmp(&b.id));
    }
    let results = RunResults { platform: platform.into(), adapter, references, cases: results };
    let regressions = match &baseline {
        Some(base) => report::regressions(base, &results),
        None => Vec::new(),
    };
    std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&results)?)?;
    let markdown = report::run_markdown(&results, &regressions);
    std::fs::write(out.join("report.md"), &markdown)?;
    println!("{markdown}");
    let bad = results.count(Status::Fail) + results.count(Status::Error);
    if bad > 0 || !regressions.is_empty() {
        bail!("{bad} cases fail or error, {} regressions", regressions.len());
    }
    Ok(())
}

fn run_case(renderer: &engine::Renderer, case: &cases::Case, refs: &Path, out: &Path, tolerances: &Tolerances) -> CaseResult {
    let tolerance = tolerances.for_case(&case.id);
    let mut result = CaseResult {
        id: case.id.clone(),
        feature: case.spec.feature.clone(),
        label: case.spec.label.clone(),
        status: Status::Error,
        tolerance,
        max_channel_diff: None,
        differing_pixels: None,
        total_pixels: None,
        message: None,
        heatmap: None,
    };
    let reference = match image::open(refs.join(format!("{}.png", case.id))) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            result.message = Some(format!("no reference: {e}"));
            return result;
        }
    };
    let port = match render_case(renderer, case) {
        Ok(img) => img,
        Err(engine::RenderError::Unsupported(what)) => {
            result.status = Status::Pending;
            result.message = Some(format!("not supported yet: {what}"));
            return result;
        }
        Err(e) => {
            result.message = Some(e.to_string());
            return result;
        }
    };
    let render_path = out.join("renders").join(format!("{}.png", case.id));
    let _ = std::fs::create_dir_all(render_path.parent().unwrap());
    let _ = port.save(&render_path);
    match compare::diff(&reference, &port, tolerance) {
        Ok(d) => {
            result.max_channel_diff = Some(d.max_channel_diff);
            result.differing_pixels = Some(d.differing_pixels);
            result.total_pixels = Some(d.total_pixels);
            if d.differing_pixels == 0 {
                result.status = Status::Pass;
            } else {
                result.status = Status::Fail;
                let name = format!("{}.png", case.id.replace('/', "__"));
                let _ = compare::heatmap(&reference, &port, &d, tolerance).save(out.join("heatmaps").join(&name));
                result.heatmap = Some(format!("heatmaps/{name}"));
            }
        }
        Err(e) => result.message = Some(e.to_string()),
    }
    result
}

fn render_case(renderer: &engine::Renderer, case: &cases::Case) -> Result<image::RgbaImage, engine::RenderError> {
    let input = case.input_path();
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut project = match ext.as_str() {
        "comp" => comp_format::load(&input).map_err(engine::RenderError::Failed)?,
        "psd" | "psb" => renderer.import_psd(&input)?,
        other => return Err(engine::RenderError::Failed(anyhow::anyhow!("unknown input type `{other}`"))),
    };
    for op in &case.spec.ops {
        renderer.apply_op(&mut project, op)?;
    }
    renderer.render(&project)
}
