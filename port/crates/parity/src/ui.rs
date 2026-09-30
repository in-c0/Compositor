//! `parity ui`: compares the Windows app's UI states and menus with the Mac's.
//!
//! Both sides render `parity/ui/states.toml`: the Mac with `ParityHarness ui` (in the references
//! artifact's `ui/`), Windows with `compositor --render-ui`. Each writes `<id>.png`,
//! `ui-info.json` and `menus.json` in the same form (parity/README.md, "UI states").
//!
//! Pixel equality isn't expected: the fonts (SF Pro against Segoe UI) and the icons (SF Symbols
//! against Phosphor, docs/port/icons.md) differ by design. So the check gates only on structure:
//! every state renders on both sides at the same size, and the menus have the same items, order,
//! separators, submenus, check marks and shortcuts once the documented mappings are applied.
//! The visual scores (SSIM on luminance, and the share of pixels far apart) are reported, not gated.

use crate::compare::Status;
use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

/// A pixel counts as "far" when a channel differs by more than this.
pub const FAR: u8 = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiResults {
    pub states: Vec<StateResult>,
    /// Menu differences that fail the check.
    pub menu_differences: Vec<String>,
    /// Items enabled on one side and disabled on the other. Reported, not gated: the port disables
    /// the items whose feature it doesn't have yet.
    pub enabled_differences: Vec<String>,
    /// Items left out of the comparison on both sides (see `SKIPPED_TITLES`), for the record.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateResult {
    pub id: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_size: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_size: Option<[u32; 2]>,
    /// Mean SSIM of the luminance (1 is identical), over the area both images cover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssim: Option<f64>,
    /// Share of pixels with a channel more than `FAR` apart, over the same area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub far_pixels: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Mac, Windows and the difference side by side, relative to the output folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<String>,
}

impl UiResults {
    pub fn load(path: &Path) -> Result<Self> {
        serde_json::from_slice(&std::fs::read(path)?).with_context(|| format!("reading {}", path.display()))
    }

    pub fn count(&self, status: Status) -> usize {
        self.states.iter().filter(|s| s.status == status).count()
    }

    pub fn passes(&self) -> bool {
        self.count(Status::Fail) == 0 && self.count(Status::Error) == 0 && self.menu_differences.is_empty()
    }
}

/// Compares `mac` (the references' `ui/`) with `windows` (`compositor --render-ui`'s output) and
/// writes `ui-results.json`, `ui-report.md` and `compare/<id>.png` into `out`.
pub fn compare(mac: &Path, windows: &Path, out: &Path) -> Result<UiResults> {
    let mac_info = read_json(&mac.join("ui-info.json"))?;
    let win_info = read_json(&windows.join("ui-info.json"))?;
    let by_id = |info: &Value| -> BTreeMap<String, Value> {
        info["states"].as_array().into_iter().flatten().filter_map(|s| Some((s["id"].as_str()?.to_string(), s.clone()))).collect()
    };
    let (mac_states, win_states) = (by_id(&mac_info), by_id(&win_info));
    // The Mac's order, then anything only Windows has.
    let mut ids: Vec<String> = mac_info["states"].as_array().into_iter().flatten().filter_map(|s| s["id"].as_str().map(String::from)).collect();
    ids.extend(win_states.keys().filter(|id| !mac_states.contains_key(*id)).cloned());
    std::fs::create_dir_all(out.join("compare"))?;
    let states = ids.iter().map(|id| compare_state(id, mac_states.get(id), win_states.get(id), mac, windows, out)).collect();

    let mut results = UiResults { states, menu_differences: Vec::new(), enabled_differences: Vec::new(), notes: Vec::new() };
    let mac_menus = read_json(&mac.join("menus.json"))?;
    let win_menus = read_json(&windows.join("menus.json"))?;
    compare_menus(&mac_menus, &win_menus, &mut results);
    std::fs::write(out.join("ui-results.json"), serde_json::to_string_pretty(&results)?)?;
    std::fs::write(out.join("ui-report.md"), markdown(&results, 2))?;
    Ok(results)
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&std::fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("parsing {}", path.display()))
}

fn compare_state(id: &str, mac: Option<&Value>, win: Option<&Value>, mac_dir: &Path, win_dir: &Path, out: &Path) -> StateResult {
    let mut r = StateResult { id: id.into(), status: Status::Error, mac_size: None, windows_size: None, ssim: None, far_pixels: None, message: None, comparison: None };
    let status = |v: Option<&Value>| v.and_then(|v| v["status"].as_str()).unwrap_or("missing").to_string();
    let (ms, ws) = (status(mac), status(win));
    let reason = |v: Option<&Value>| v.and_then(|v| v["error"].as_str().or(v["reason"].as_str())).unwrap_or("").to_string();
    if ms != "ok" {
        r.message = Some(format!("the Mac didn't render it ({ms}): {}", reason(mac)));
        return r;
    }
    match ws.as_str() {
        "ok" => {}
        "pending" => {
            r.status = Status::Pending;
            r.message = Some(reason(win));
            return r;
        }
        other => {
            r.status = Status::Fail;
            r.message = Some(format!("Windows didn't render it ({other}): {}", reason(win)));
            return r;
        }
    }
    let load = |dir: &Path| image::open(dir.join(format!("{id}.png"))).map(|i| i.to_rgba8());
    let (a, b) = match (load(mac_dir), load(win_dir)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            r.message = Some(format!("couldn't read a PNG: {e}"));
            return r;
        }
    };
    r.mac_size = Some([a.width(), a.height()]);
    r.windows_size = Some([b.width(), b.height()]);
    let scores = Scores::measure(&a, &b);
    r.ssim = Some(scores.ssim);
    r.far_pixels = Some(scores.far);
    let name = format!("compare/{}.png", id.replace('/', "__"));
    if side_by_side(&a, &b).save(out.join(&name)).is_ok() {
        r.comparison = Some(name);
    }
    if a.dimensions() == b.dimensions() {
        r.status = Status::Pass;
    } else {
        r.status = Status::Fail;
        r.message = Some(format!("size differs: Mac {} × {}, Windows {} × {}", a.width(), a.height(), b.width(), b.height()));
    }
    r
}

struct Scores {
    ssim: f64,
    far: f64,
}

impl Scores {
    /// Over the top-left area both images cover.
    fn measure(a: &RgbaImage, b: &RgbaImage) -> Self {
        let (w, h) = (a.width().min(b.width()), a.height().min(b.height()));
        let luma = |img: &RgbaImage| -> Vec<f32> {
            let mut v = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let p = img.get_pixel(x, y);
                    v.push(0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32);
                }
            }
            v
        };
        let mut far = 0u64;
        for y in 0..h {
            for x in 0..w {
                let (p, q) = (a.get_pixel(x, y), b.get_pixel(x, y));
                if (0..4).any(|c| p[c].abs_diff(q[c]) > FAR) {
                    far += 1;
                }
            }
        }
        let total = (w as u64 * h as u64).max(1);
        Scores { ssim: ssim(&luma(a), &luma(b), w as usize, h as usize), far: far as f64 / total as f64 }
    }
}

/// Mean structural similarity (Wang et al. 2004): an 11-tap Gaussian window with σ 1.5,
/// K1 0.01, K2 0.03, on 0–255 values. Edges clamp.
pub fn ssim(x: &[f32], y: &[f32], w: usize, h: usize) -> f64 {
    if w == 0 || h == 0 {
        return 1.0;
    }
    let kernel: Vec<f32> = {
        let k: Vec<f32> = (-5i32..=5).map(|i| (-(i * i) as f32 / (2.0 * 1.5 * 1.5)).exp()).collect();
        let sum: f32 = k.iter().sum();
        k.into_iter().map(|v| v / sum).collect()
    };
    let blur = |src: &[f32]| -> Vec<f32> {
        let mut tmp = vec![0f32; w * h];
        for row in 0..h {
            for col in 0..w {
                let mut acc = 0.0;
                for (i, k) in kernel.iter().enumerate() {
                    let c = (col as i32 + i as i32 - 5).clamp(0, w as i32 - 1) as usize;
                    acc += k * src[row * w + c];
                }
                tmp[row * w + col] = acc;
            }
        }
        let mut dst = vec![0f32; w * h];
        for row in 0..h {
            for col in 0..w {
                let mut acc = 0.0;
                for (i, k) in kernel.iter().enumerate() {
                    let r = (row as i32 + i as i32 - 5).clamp(0, h as i32 - 1) as usize;
                    acc += k * tmp[r * w + col];
                }
                dst[row * w + col] = acc;
            }
        }
        dst
    };
    let product = |p: &[f32], q: &[f32]| p.iter().zip(q).map(|(a, b)| a * b).collect::<Vec<f32>>();
    let (mx, my) = (blur(x), blur(y));
    let (sxx, syy, sxy) = (blur(&product(x, x)), blur(&product(y, y)), blur(&product(x, y)));
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let mut total = 0.0f64;
    for i in 0..w * h {
        let (ux, uy) = (mx[i] as f64, my[i] as f64);
        let vx = (sxx[i] as f64 - ux * ux).max(0.0);
        let vy = (syy[i] as f64 - uy * uy).max(0.0);
        let cov = sxy[i] as f64 - ux * uy;
        total += ((2.0 * ux * uy + c1) * (2.0 * cov + c2)) / ((ux * ux + uy * uy + c1) * (vx + vy + c2));
    }
    total / (w * h) as f64
}

/// Mac, Windows and the difference next to each other: side by side for tall states, stacked for
/// wide ones. Where the sizes differ, the area only one side covers is magenta. The difference
/// is the Mac's image dimmed, with pixels over 8/255 in blue and over `FAR` in yellow to red.
pub fn side_by_side(a: &RgbaImage, b: &RgbaImage) -> RgbaImage {
    let (w, h) = (a.width().max(b.width()), a.height().max(b.height()));
    let gap = 6;
    let stacked = w > 2 * h;
    let (ow, oh) = if stacked { (w, 3 * h + 2 * gap) } else { (3 * w + 2 * gap, h) };
    let mut out = RgbaImage::from_pixel(ow, oh, Rgba([96, 96, 96, 255]));
    let offset = |i: u32| if stacked { (0, i * (h + gap)) } else { (i * (w + gap), 0) };
    let magenta = Rgba([255, 0, 255, 255]);
    for y in 0..h {
        for x in 0..w {
            let pa = (x < a.width() && y < a.height()).then(|| *a.get_pixel(x, y));
            let pb = (x < b.width() && y < b.height()).then(|| *b.get_pixel(x, y));
            let (x0, y0) = offset(0);
            out.put_pixel(x0 + x, y0 + y, pa.unwrap_or(magenta));
            let (x1, y1) = offset(1);
            out.put_pixel(x1 + x, y1 + y, pb.unwrap_or(magenta));
            let diff = match (pa, pb) {
                (Some(p), Some(q)) => {
                    let d = (0..4).map(|c| p[c].abs_diff(q[c])).max().unwrap_or(0);
                    let base = ((0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) * 0.3) as u8;
                    if d > FAR {
                        let t = ((d - FAR) as f32 / 96.0).min(1.0);
                        Rgba([255, (220.0 * (1.0 - t)) as u8, 0, 255])
                    } else if d > 8 {
                        Rgba([base, base, 200, 255])
                    } else {
                        Rgba([base, base, base, 255])
                    }
                }
                _ => magenta,
            };
            let (x2, y2) = offset(2);
            out.put_pixel(x2 + x, y2 + y, diff);
        }
    }
    out
}

// MARK: Menus

/// Items macOS adds to an app's menus on its own. They aren't the app's, so neither side's copy is
/// compared (Windows has none of them).
pub const SKIPPED_TITLES: [&str; 8] =
    ["AutoFill", "Start Dictation…", "Emoji & Symbols", "Writing Tools", "Enter Full Screen", "Exit Full Screen", "Show Tab Bar", "Show All Tabs"];

/// Menus whose contents the operating system supplies on both platforms: only their place in the
/// menu bar is compared.
pub const SYSTEM_MENUS: [&str; 2] = ["Window", "Help"];

/// The Mac's application menu. Windows has no such menu: its items are the system's (Services,
/// Hide, Quit) or Mac-only (About and Check for Updates go through the app menu on the Mac; the
/// port has no updater), and Windows puts Exit at the foot of File instead.
pub const MAC_APP_MENU: &str = "Compositor";

/// An item after the mappings: what is compared.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    separator: bool,
    title: String,
    shortcut: Option<String>,
    enabled: bool,
    checked: bool,
    items: Option<Vec<Entry>>,
}

impl Entry {
    fn label(&self) -> String {
        if self.separator { "(separator)".into() } else { format!("“{}”", self.title) }
    }
}

/// Leaves out system, hidden and alternate items and the ones in `SKIPPED_TITLES`, then the
/// separators that leaves at either end or doubled, as a menu would look without those items.
fn normalize(items: &Value) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for item in items.as_array().into_iter().flatten() {
        let truthy = |k: &str| item.get(k).is_some_and(|v| v.as_bool() != Some(false) && !v.is_null());
        if truthy("system") || truthy("hidden") || truthy("alternate") {
            continue;
        }
        if item["separator"].as_bool() == Some(true) {
            if out.last().is_some_and(|e| !e.separator) {
                out.push(Entry { separator: true, title: String::new(), shortcut: None, enabled: true, checked: false, items: None });
            }
            continue;
        }
        let title = item["title"].as_str().unwrap_or("").to_string();
        if SKIPPED_TITLES.contains(&title.as_str()) {
            continue;
        }
        let children = item.get("items").or(item.get("children"));
        out.push(Entry {
            separator: false,
            title,
            shortcut: item["shortcut"].as_str().map(String::from),
            enabled: item["enabled"].as_bool().unwrap_or(false),
            checked: item["checked"].as_bool() == Some(true) || item["checked"].as_str() == Some("mixed"),
            items: children.map(normalize),
        });
    }
    while out.last().is_some_and(|e| e.separator) {
        out.pop();
    }
    out
}

/// Aligns two item lists by title (a longest common subsequence) and lists what differs.
fn diff_items(path: &str, mac: &[Entry], win: &[Entry], gating: &mut Vec<String>, enabled: &mut Vec<String>) {
    let key = |e: &Entry| if e.separator { "\u{0}sep".to_string() } else { e.title.clone() };
    let (n, m) = (mac.len(), win.len());
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if key(&mac[i]) == key(&win[j]) { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut missing: Vec<(usize, &Entry)> = Vec::new();
    let mut extra: Vec<(usize, &Entry)> = Vec::new();
    while i < n || j < m {
        if i < n && j < m && key(&mac[i]) == key(&win[j]) {
            compare_entry(path, &mac[i], &win[j], gating, enabled);
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            extra.push((j, &win[j]));
            j += 1;
        } else {
            missing.push((i, &mac[i]));
            i += 1;
        }
    }
    // An item on both sides but in a different place shows up as missing and extra: call it moved.
    for (mi, e) in &missing {
        if let Some(pos) = extra.iter().position(|(_, x)| key(x) == key(e)) {
            let (wi, x) = extra.remove(pos);
            gating.push(format!("{path}: {} is item {} on the Mac and {} on Windows", e.label(), mi + 1, wi + 1));
            compare_entry(path, e, x, gating, enabled);
        } else {
            gating.push(format!("{path}: {} (item {}) is missing on Windows", e.label(), mi + 1));
        }
    }
    for (wi, x) in extra {
        gating.push(format!("{path}: {} (item {}) is only on Windows", x.label(), wi + 1));
    }
}

fn compare_entry(path: &str, mac: &Entry, win: &Entry, gating: &mut Vec<String>, enabled: &mut Vec<String>) {
    if mac.separator {
        return;
    }
    let here = format!("{path} > {}", mac.title);
    if mac.shortcut != win.shortcut {
        let s = |v: &Option<String>| v.clone().unwrap_or_else(|| "none".into());
        gating.push(format!("{here}: shortcut {} on the Mac, {} on Windows", s(&mac.shortcut), s(&win.shortcut)));
    }
    if mac.checked != win.checked {
        let s = |on: bool| if on { "checked" } else { "unchecked" };
        gating.push(format!("{here}: {} on the Mac, {} on Windows", s(mac.checked), s(win.checked)));
    }
    if mac.enabled != win.enabled && mac.items.is_none() {
        let s = |on: bool| if on { "enabled" } else { "disabled" };
        enabled.push(format!("{here}: {} on the Mac, {} on Windows", s(mac.enabled), s(win.enabled)));
    }
    match (&mac.items, &win.items) {
        (Some(a), Some(b)) => diff_items(&here, a, b, gating, enabled),
        (Some(_), None) => gating.push(format!("{here}: a submenu on the Mac, a plain item on Windows")),
        (None, Some(_)) => gating.push(format!("{here}: a plain item on the Mac, a submenu on Windows")),
        (None, None) => {}
    }
}

fn compare_menus(mac: &Value, win: &Value, results: &mut UiResults) {
    let (gating, enabled) = (&mut results.menu_differences, &mut results.enabled_differences);
    let menus = |v: &Value| -> Vec<(String, Value)> {
        v["mainMenu"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["title"].as_str() != Some(MAC_APP_MENU))
            .map(|m| (m["title"].as_str().unwrap_or("").to_string(), m.get("items").or(m.get("children")).cloned().unwrap_or(Value::Null)))
            .collect()
    };
    let (mac_menus, win_menus) = (menus(mac), menus(win));
    if mac_menus.is_empty() {
        gating.push("The Mac's menus.json has no main menu".into());
    }
    // The menu bar itself, as a list of titles.
    let bar = |list: &[(String, Value)]| -> Vec<Entry> {
        list.iter().map(|(t, _)| Entry { separator: false, title: t.clone(), shortcut: None, enabled: true, checked: false, items: None }).collect()
    };
    diff_items("Menu bar", &bar(&mac_menus), &bar(&win_menus), gating, enabled);
    for (title, items) in &mac_menus {
        if SYSTEM_MENUS.contains(&title.as_str()) {
            continue;
        }
        if let Some((_, other)) = win_menus.iter().find(|(t, _)| t == title) {
            diff_items(title, &normalize(items), &normalize(other), gating, enabled);
        }
    }
    // The Layers list's row menus, matched by document and row.
    let rows = |v: &Value| -> BTreeMap<(String, u64), Value> {
        v["layerContextMenus"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| Some(((r["document"].as_str()?.to_string(), r["row"].as_u64()?), r.clone())))
            .collect()
    };
    let (mac_rows, win_rows) = (rows(mac), rows(win));
    for ((doc, row), m) in &mac_rows {
        let path = format!("Layer menu, {doc} row {row} ({})", m["layer"].as_str().unwrap_or("?"));
        if let Some(e) = m["error"].as_str() {
            gating.push(format!("{path}: the Mac couldn't open it: {e}"));
            continue;
        }
        let Some(w) = win_rows.get(&(doc.clone(), *row)) else {
            gating.push(format!("{path}: missing on Windows"));
            continue;
        };
        if m["layer"] != w["layer"] {
            gating.push(format!("{path}: the row is “{}” on Windows", w["layer"].as_str().unwrap_or("?")));
        }
        diff_items(&path, &normalize(&m["items"]), &normalize(&w["items"]), gating, enabled);
    }
    for (doc, row) in win_rows.keys().filter(|k| !mac_rows.contains_key(*k)) {
        gating.push(format!("Layer menu, {doc} row {row}: only on Windows"));
    }
    results.notes.push(format!(
        "Not compared: the Mac's {MAC_APP_MENU} menu, the contents of the {} menus, items marked system, hidden or alternate, and macOS's own {}.",
        SYSTEM_MENUS.join(" and "),
        SKIPPED_TITLES.join(", ")
    ));
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Pass => "pass",
        Status::Fail => "**fail**",
        Status::Pending => "pending",
        Status::Error => "**error**",
    }
}

/// The UI section: a table of states, then every menu difference. `level` is the heading depth.
pub fn markdown(r: &UiResults, level: usize) -> String {
    let h = "#".repeat(level);
    let mut md = String::new();
    let _ = writeln!(md, "{h} UI\n");
    let _ = writeln!(
        md,
        "Each UI state in `parity/ui/states.toml`, rendered by the Mac app's views and by the Windows app. A state passes when both \
         render it at the same size; fonts (SF Pro against Segoe UI) and icons (SF Symbols against Phosphor) keep the pixels apart, so \
         the scores are reported, not gated. SSIM is on luminance (1 is identical); \"far\" is the share of pixels with a channel more \
         than {FAR}/255 apart.\n"
    );
    let _ = writeln!(
        md,
        "{} of {} states pass, {} fail, {} pending, {} errors. {} menu differences{}.\n",
        r.count(Status::Pass),
        r.states.len(),
        r.count(Status::Fail),
        r.count(Status::Pending),
        r.count(Status::Error),
        r.menu_differences.len(),
        if r.enabled_differences.is_empty() { String::new() } else { format!(", {} enabled-state differences (not gated)", r.enabled_differences.len()) }
    );
    let _ = writeln!(md, "| State | Result | Mac | Windows | SSIM | Far | Detail |\n| --- | --- | --- | --- | --- | --- | --- |");
    let size = |s: Option<[u32; 2]>| s.map(|[w, h]| format!("{w} × {h}")).unwrap_or_default();
    for s in &r.states {
        let _ = writeln!(
            md,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            s.id,
            status_word(s.status),
            size(s.mac_size),
            size(s.windows_size),
            s.ssim.map(|v| format!("{v:.3}")).unwrap_or_default(),
            s.far_pixels.map(|v| format!("{:.1}%", v * 100.0)).unwrap_or_default(),
            s.message.as_deref().unwrap_or("").replace('|', "\\|")
        );
    }
    let _ = writeln!(md, "\n{h}# Menus\n");
    if r.menu_differences.is_empty() {
        let _ = writeln!(md, "The menus match: items, order, separators, submenus, check marks and shortcuts (Ctrl written as ⌘).\n");
    } else {
        for d in &r.menu_differences {
            let _ = writeln!(md, "- {d}");
        }
        md.push('\n');
    }
    for note in &r.notes {
        let _ = writeln!(md, "{note}\n");
    }
    if !r.enabled_differences.is_empty() {
        let _ = writeln!(md, "<details><summary>Enabled on one side only ({})</summary>\n", r.enabled_differences.len());
        for d in &r.enabled_differences {
            let _ = writeln!(md, "- {d}");
        }
        let _ = writeln!(md, "\n</details>\n");
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identical_images_score_one() {
        let a = RgbaImage::from_fn(40, 30, |x, y| Rgba([(x * 6) as u8, (y * 8) as u8, 90, 255]));
        let s = Scores::measure(&a, &a);
        assert!((s.ssim - 1.0).abs() < 1e-9);
        assert_eq!(s.far, 0.0);
        let b = RgbaImage::from_pixel(40, 30, Rgba([0, 0, 0, 255]));
        assert!(Scores::measure(&a, &b).ssim < 0.5);
    }

    #[test]
    fn menus_skip_system_items_and_trim_separators() {
        let mac = json!([
            { "title": "Undo", "enabled": false, "shortcut": "⌘Z" },
            { "separator": true },
            { "title": "Cut", "enabled": true, "shortcut": "⌘X" },
            { "separator": true },
            { "title": "AutoFill", "enabled": true, "items": [] },
            { "title": "Emoji & Symbols", "enabled": true, "hidden": true },
        ]);
        let win = json!([
            { "title": "Undo", "enabled": false, "shortcut": "⌘Z" },
            { "separator": true },
            { "title": "Cut", "enabled": false, "shortcut": "⌘X" },
            { "separator": true, "system": true },
            { "title": "Exit", "enabled": true, "system": true },
        ]);
        let (mut gating, mut enabled) = (Vec::new(), Vec::new());
        diff_items("Edit", &normalize(&mac), &normalize(&win), &mut gating, &mut enabled);
        assert!(gating.is_empty(), "{gating:?}");
        assert_eq!(enabled.len(), 1);
    }

    #[test]
    fn menus_report_moves_shortcuts_and_missing_items() {
        let mac = json!([{ "title": "A", "enabled": true }, { "title": "B", "enabled": true, "shortcut": "⌘B" }, { "title": "C", "enabled": true }]);
        let win = json!([{ "title": "B", "enabled": true, "shortcut": "⇧⌘B" }, { "title": "A", "enabled": true }]);
        let (mut gating, mut enabled) = (Vec::new(), Vec::new());
        diff_items("M", &normalize(&mac), &normalize(&win), &mut gating, &mut enabled);
        assert_eq!(gating.len(), 3, "{gating:?}");
        assert!(gating.iter().any(|g| g.contains("shortcut ⌘B on the Mac, ⇧⌘B on Windows")));
        assert!(gating.iter().any(|g| g.contains("“C” (item 3) is missing on Windows")));
        assert!(gating.iter().any(|g| g.contains("item 1 on the Mac and 2 on Windows") || g.contains("item 2 on the Mac and 1 on Windows")));
    }
}
