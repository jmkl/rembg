//! CompNode — a small node-based image processing studio.
//!
//! The crate is split by responsibility:
//!
//! - [`style`] owns every visual constant and the egui/egui theme setup.
//! - [`model`] contains graph data (nodes, previews, viewer layers).
//! - [`catalog`] is the single node registry used by menus and the graph view.
//! - [`nodes`] contains one body UI module per node kind.
//! - [`graph`] contains graph operations such as connecting, invalidating and
//!   refreshing previews.
//! - [`memory`] bounds full-resolution CPU images and reloads them on demand.
//! - [`editor`] implements egui-snarl's `SnarlViewer` for the graph.
//! - [`session`], [`cache`] and [`worker`] cover persistence and inference.
//! - [`settings`] and [`settings_window`] own persisted app configuration.
//! - [`app`] wires the application together, including autosave and menus.

mod app;
mod assets;
mod cache;
mod catalog;
mod editor;
mod eval;
mod gpaste_app;
mod graph;
mod graph_menu;
mod image_canvas;
mod layer_cache;
mod memory;
mod model;
mod nodes;
mod ops;
mod params;
mod session;
mod settings;
mod settings_window;
mod style;
#[cfg(test)]
mod tests;
mod title_bar;
mod widgets;
mod worker;

pub use app::CompNodeApp;
pub use app::run;
pub use image_canvas::ImageCanvas;
