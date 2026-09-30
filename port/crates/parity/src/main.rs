//! `parity`: renders the corpus with the port and checks it against the Mac app's references.

mod affected;
mod cases;
mod compare;
mod corpus_gen;
mod export_check;
mod projects;
mod report;
mod roundtrip;

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
    /// Compare the project the port imports from each Photoshop case with the Mac's `.comp`.
    CompareProjects {
        #[arg(long, default_value = "parity/corpus")]
        corpus: PathBuf,
        /// References from the Mac harness: `<case id>.comp` and `harness-info.json`.
        #[arg(long)]
        refs: PathBuf,
        /// Case id globs; every Photoshop case when omitted.
        #[arg(long = "case")]
        cases: Vec<String>,
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
        Command::CompareProjects { corpus, refs, cases } => projects::compare_projects(&corpus, &refs, &cases),
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
    let references: serde_json::Value = std::fs::read(refs.join("harness-info.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null);
    // Cases the Mac app refused to open, with its message: the port has to refuse them too.
    let rejected: std::collections::HashMap<String, String> = references["cases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["status"] == "error")
        .filter_map(|c| Some((c["id"].as_str()?.to_string(), c["error"].as_str().unwrap_or("").to_string())))
        .collect();
    let mut results: Vec<CaseResult> =
        cases.par_iter().map(|case| run_case(&renderer, case, refs, out, &tolerances, rejected.get(&case.id))).collect();
    let scratch = out.join("roundtrip");
    std::fs::create_dir_all(&scratch)?;
    results.extend(cases.par_iter().filter_map(|case| roundtrip::check(&case.id, refs, &scratch)).collect::<Vec<_>>());
    // Export: the PNG's metadata for every export case, and the JPEG where the case asks for one.
    let out_dir = out;
    results.extend(
        cases
            .par_iter()
            .filter(|case| case.spec.feature == "export")
            .flat_map(|case| {
                let mut out = Vec::new();
                let Ok(project) = build_project(&renderer, case) else { return out };
                let Ok(image) = renderer.render(&project) else { return out };
                let resolution = project.manifest.resolution.unwrap_or(72.0);
                if let Ok(png) = engine::export::png(&image, resolution) {
                    out.push(export_check::png_metadata(&case.id, &case.spec.feature, &png, refs));
                }
                if let Some(options) = &case.spec.jpeg {
                    if let Ok(jpeg) = renderer.export_jpeg(&project, options) {
                        out.push(export_check::jpeg_tables(&case.id, &case.spec.feature, &jpeg, refs));
                        out.push(export_check::jpeg(&case.id, &case.spec.feature, &jpeg, refs, out_dir, &tolerances));
                    }
                }
                out
            })
            .collect::<Vec<_>>(),
    );
    // Cases the Mac saved a project for: compare the port's project with it.
    results.extend(
        cases
            .par_iter()
            .filter(|case| refs.join(format!("{}.comp", case.id)).is_dir())
            .filter_map(|case| match build_project(&renderer, case) {
                Ok(project) => {
                    // Layer and mask pixels match byte for byte, except where an override sets
                    // a structure limit.
                    let limit = tolerances.for_structure(&case.id);
                    roundtrip::check_structure(&case.id, &case.spec.feature, &project, refs, limit.as_ref())
                }
                Err(_) => None,
            })
            .collect::<Vec<_>>(),
    );
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

fn run_case(
    renderer: &engine::Renderer,
    case: &cases::Case,
    refs: &Path,
    out: &Path,
    tolerances: &Tolerances,
    mac_rejected: Option<&String>,
) -> CaseResult {
    let limit = tolerances.for_case(&case.id);
    let tolerance = limit.max_channel_diff;
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
    if let Some(mac_error) = mac_rejected {
        match render_case(renderer, case) {
            Err(engine::RenderError::Unsupported(what)) => {
                result.status = Status::Pending;
                result.message = Some(format!("not supported yet: {what}"));
            }
            Err(e) => {
                result.status = Status::Pass;
                result.message = Some(format!("both refuse it. Mac: {mac_error} Port: {e}"));
            }
            Ok(_) => {
                result.status = Status::Fail;
                result.message = Some(format!("the Mac refuses this input ({mac_error}) but the port rendered it"));
            }
        }
        return result;
    }
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
            if d.differing_pixels <= limit.max_pixels_over {
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
    let project = build_project(renderer, case)?;
    renderer.render(&project)
}

/// The case's input, opened (or imported) and with its ops applied.
fn build_project(renderer: &engine::Renderer, case: &cases::Case) -> Result<comp_format::Project, engine::RenderError> {
    let input = case.input_path();
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut project = match ext.as_str() {
        "comp" => comp_format::load(&input).map_err(engine::RenderError::Failed)?,
        "psd" | "psb" => renderer.import_psd(&input)?,
        _ => renderer.import_image(&input, case.spec.raw.as_ref())?,
    };
    for op in &case.spec.ops {
        renderer.apply_op(&mut project, op)?;
    }
    Ok(project)
}
