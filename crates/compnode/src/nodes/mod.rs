//! One body UI per node kind.
//!
//! Each module exposes:
//! - `create(snarl, pos)` for the graph context menu.
//! - `show_body(editor, node, ui, snarl, width)` for the node body.
//!
//! Register new modules in [`crate::catalog`], not in a central match.

pub(crate) mod face_restore;
pub(crate) mod filter;
pub(crate) mod image;
pub(crate) mod output;
pub(crate) mod remove_bg;
pub(crate) mod remove_bgv2;

/// File picker filter used by Image nodes.
pub(crate) const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];
