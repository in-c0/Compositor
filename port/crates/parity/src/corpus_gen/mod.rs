//! Builds the parity corpus. Every byte comes from this code and fixed seeds, so running it twice
//! gives the same files, and CI checks the committed corpus against a fresh run.

mod builder;
mod images;
mod import_files;
mod import_suite;
mod psd_suite;
mod suites;

use anyhow::Result;
use std::path::Path;

pub use builder::CaseWriter;

pub fn generate(out: &Path) -> Result<()> {
    if out.exists() {
        std::fs::remove_dir_all(out)?;
    }
    std::fs::create_dir_all(out)?;
    let mut w = CaseWriter::new(out);
    suites::all(&mut w)?;
    psd_suite::psd(&mut w)?;
    import_suite::import(&mut w)?;
    eprintln!("wrote {} cases to {}", w.count, out.display());
    Ok(())
}
