//! Icon glyphs (Lucide, bundled in `assets/fonts/Symbols.ttf`).
//!
//! Icons live in their own font family so the UI font can never shadow the
//! private-use codepoints. Always draw them through [`rich`] / [`font`].

use egui::{FontFamily, FontId, RichText};

pub(crate) const FAMILY: &str = "icons";

pub(crate) fn family() -> FontFamily {
    FontFamily::Name(FAMILY.into())
}

pub(crate) fn font(size: f32) -> FontId {
    FontId::new(size, family())
}

/// An icon as `RichText` in the icon font.
pub(crate) fn rich(icon: char, size: f32) -> RichText {
    RichText::new(icon.to_string()).font(font(size))
}

pub(crate) const DOWNLOAD: char = '\u{E001}';
pub(crate) const COPY: char = '\u{E004}';
pub(crate) const NETWORK: char = '\u{E006}';
pub(crate) const ZOOM_IN: char = '\u{E007}';
pub(crate) const TRASH: char = '\u{E00A}';
pub(crate) const PENCIL: char = '\u{E00B}';
pub(crate) const PLUS: char = '\u{E00C}';
pub(crate) const REFRESH: char = '\u{E00F}';
pub(crate) const INFO: char = '\u{E012}';
pub(crate) const TRIANGLE_ALERT: char = '\u{E013}';
pub(crate) const SETTINGS: char = '\u{E016}';
pub(crate) const MENU: char = '\u{E017}';
pub(crate) const ARROW_LEFT: char = '\u{E018}';
pub(crate) const ARROW_RIGHT: char = '\u{E019}';
pub(crate) const CHEVRON_RIGHT: char = '\u{E01B}';
pub(crate) const CHEVRON_DOWN: char = '\u{E01C}';
pub(crate) const FILTER: char = '\u{E01D}';
pub(crate) const EYE: char = '\u{E01E}';
pub(crate) const EYE_OFF: char = '\u{E01F}';
pub(crate) const SAVE: char = '\u{E022}';
pub(crate) const FOLDER_OPEN: char = '\u{E024}';
pub(crate) const CHECK: char = '\u{2713}';
pub(crate) const X: char = '\u{2717}';
