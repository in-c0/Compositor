//! Camera RAW: the develop sheet's settings and `CIRAWFilter`'s develop.

use crate::{ImportError, Layer, Result};

/// The develop sheet's controls (`RawDevelopSettings`). Temperature and tint left `None` keep the
/// camera's own white balance, as the sheet opens with it.
#[derive(Clone, Debug, Default)]
pub struct RawSettings {
    pub exposure: Option<f32>,
    pub temperature: Option<f32>,
    pub tint: Option<f32>,
    pub boost: Option<f32>,
}

/// `RawImporter.matches`: the extensions macOS types as camera RAW images.
pub(crate) fn matches(extension: &str) -> bool {
    matches!(
        extension,
        "dng" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "srf" | "sr2" | "raf" | "orf" | "rw2" | "raw" | "pef" | "srw" | "x3f"
            | "erf" | "mrw" | "mos" | "3fr" | "fff" | "iiq" | "dcr" | "kdc" | "rwl" | "mef" | "k25" | "dcs" | "gpr" | "nksc"
    )
}

pub(crate) fn develop(_data: &[u8], _settings: RawSettings) -> Result<Layer> {
    Err(ImportError::NotPorted("camera RAW develop".into()))
}
