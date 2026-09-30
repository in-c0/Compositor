//! Reads and writes Compositor `.comp` packages (manifest versions 1–11).
//!
//! The types mirror the Swift `Codable` structs in `Compositor/IO/ProjectStore.swift` and
//! `Compositor/Document/*`. Swift's synthesized decoding requires every non-optional key, so the
//! non-`Option` fields here are exactly the keys the Mac app requires; `Option` fields are omitted
//! when absent, as Swift's `encodeIfPresent` does.

mod json;
mod model;
mod package;
mod validate;

pub use json::to_swift_json;
pub use model::*;
pub use package::{Asset, GrayImage, Project, RgbaImage, encode_png, load, save};
pub use validate::validate;
