//! The graph-view boundary.
//!
//! The editor logic (selection, undo, evaluation, node bodies, wiring rules) is
//! independent of the widget that draws the node graph. This module defines what
//! such a widget has to do. The egui-snarl implementation lives in [`snarl`];
//! it is the only module besides [`crate::nodegraph`] allowed to name the
//! `egui_snarl` crate.
//!
//! # Writing another backend
//!
//! Implement [`GraphView`] and return it from [`default_view`]. Per frame, in
//! [`GraphView::show`], the view must:
//!
//! 1. **Draw** the canvas (pan/zoom, background) and, for every node, a frame
//!    with a header, input pins on one side, output pins on the other and a
//!    body, plus the wires between connected pins. The *contents* come from the
//!    editor, so every backend looks and behaves the same:
//!    - header: [`GraphEditor::draw_node_header`]
//!    - body: [`GraphEditor::draw_node_body`]
//!    - pin look: [`GraphEditor::input_pin_style`], [`GraphEditor::output_pin_style`]
//!    - node menu: [`GraphEditor::draw_node_menu`]
//!    - frames: `style::node_frame` and `style::node_header_frame`
//! 2. **Report geometry** into [`GraphEditor::geometry`] (screen space): call
//!    [`ViewGeometry::begin_frame`] at the start of the frame,
//!    [`ViewGeometry::set_transform`] with the canvas-to-screen transform, and
//!    [`GraphEditor::node_laid_out`] for each node.
//! 3. **Forward intent**: a new wire is [`GraphEditor::on_connect`], a removed
//!    one [`GraphEditor::on_disconnect`], dragging a wire off a pin
//!    [`GraphEditor::on_drop_inputs`] / [`GraphEditor::on_drop_outputs`]. Never
//!    edit wires directly: the editor records undo steps and invalidates caches.
//! 4. **Honour** [`GraphEditor::pan_locked`]: no panning while a wire cut is
//!    being dragged.
//! 5. **Own selection rectangles** if the widget has them, and expose the
//!    result through [`GraphView::selection`]. Clicks on nodes are handled by
//!    the editor itself (see `node_laid_out`).
//!
//! Then replace the storage inside `crate::nodegraph::NodeGraph` if the new
//! widget needs its own data structure; nothing else in the crate changes.

use std::collections::HashMap;

use egui::emath::TSTransform;
use egui::{Color32, Rect, Stroke};

use crate::editor::GraphEditor;
use crate::model::Node;
use crate::nodegraph::{InPinId, NodeGraph, NodeId, OutPinId};
use crate::style::pal;

mod snarl;

/// A node-graph widget. See the [module documentation](self) for the contract.
pub(crate) trait GraphView {
    /// Draw the graph and run its pointer interaction. The returned response
    /// covers the canvas; the app uses it to detect clicks on empty space.
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        graph: &mut NodeGraph<Node>,
        editor: &mut GraphEditor,
    ) -> egui::Response;

    /// Nodes the widget itself has selected (for example with a selection
    /// rectangle). Read after [`GraphView::show`].
    fn selection(&self, ctx: &egui::Context) -> Vec<NodeId>;

    /// Forget pan, zoom and selection: the graph was replaced (new file).
    fn reset(&mut self);

    /// Changes on every [`GraphView::reset`]; `0` for a fresh view.
    fn generation(&self) -> u64;
}

/// The view used by the application. This is the one place to switch backends.
pub(crate) fn default_view() -> Box<dyn GraphView> {
    Box::new(snarl::SnarlView::new())
}

/// What the active view reported on its last frame, in screen space. The
/// editor uses it for wire cutting and for placing new nodes under the cursor.
#[derive(Clone, Debug)]
pub(crate) struct ViewGeometry {
    /// Current canvas scale.
    pub(crate) zoom: f32,
    /// Canvas-to-screen transform.
    pub(crate) to_global: TSTransform,
    pub(crate) node_rects: HashMap<NodeId, Rect>,
    /// Screen-space Y of each pin row, recorded while the node is drawn.
    pub(crate) input_pin_y: HashMap<InPinId, f32>,
    pub(crate) output_pin_y: HashMap<OutPinId, f32>,
}

impl Default for ViewGeometry {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            to_global: TSTransform::IDENTITY,
            node_rects: HashMap::new(),
            input_pin_y: HashMap::new(),
            output_pin_y: HashMap::new(),
        }
    }
}

impl ViewGeometry {
    /// Pin heights are recorded again while this frame's nodes are drawn.
    pub(crate) fn begin_frame(&mut self) {
        self.input_pin_y.clear();
        self.output_pin_y.clear();
    }

    pub(crate) fn set_transform(&mut self, to_global: TSTransform) {
        self.to_global = to_global;
        self.zoom = to_global.scaling.max(0.05);
    }

    /// Forget everything (the graph was replaced).
    pub(crate) fn clear(&mut self) {
        self.node_rects.clear();
        self.input_pin_y.clear();
        self.output_pin_y.clear();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PinShape {
    Circle,
    Square,
}

/// How a pin and its wires are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PinStyle {
    pub(crate) shape: PinShape,
    pub(crate) fill: Color32,
    pub(crate) stroke: Stroke,
    pub(crate) wire: Color32,
}

impl PinStyle {
    fn new(shape: PinShape) -> Self {
        Self {
            shape,
            fill: pal::SOCKET,
            stroke: Stroke::new(1.0, pal::SOCKET_STROKE),
            wire: pal::SOCKET,
        }
    }

    pub(crate) fn circle() -> Self {
        Self::new(PinShape::Circle)
    }

    pub(crate) fn square() -> Self {
        Self::new(PinShape::Square)
    }

    pub(crate) fn with_fill(mut self, fill: Color32) -> Self {
        self.fill = fill;
        self
    }

    pub(crate) fn with_stroke(mut self, stroke: Stroke) -> Self {
        self.stroke = stroke;
        self
    }

    pub(crate) fn with_wire_color(mut self, wire: Color32) -> Self {
        self.wire = wire;
        self
    }
}

/// Dot grid whose dots keep a constant on-screen size. Currently unused: the
/// canvas is a flat fill (see `snarl_style`), but a backend can call it.
#[allow(dead_code)]
pub(crate) fn paint_dot_grid(painter: &egui::Painter, viewport: &egui::Rect, zoom: f32) {
    let zoom = zoom.max(0.05);
    let mut spacing = 22.0;
    while spacing * zoom < 40.0 {
        spacing *= 2.0;
    }
    while spacing * zoom > 80.0 {
        spacing *= 0.5;
    }

    let radius = (1.1 / zoom).clamp(0.45, 2.0);
    let x0 = (viewport.left() / spacing).floor() * spacing;
    let y0 = (viewport.top() / spacing).floor() * spacing;

    let mut y = y0;
    while y <= viewport.bottom() {
        let mut x = x0;
        while x <= viewport.right() {
            painter.circle_filled(egui::pos2(x, y), radius, pal::BG);
            x += spacing;
        }
        y += spacing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_tracks_the_transform_and_clamps_the_zoom() {
        let mut geometry = ViewGeometry::default();
        geometry.set_transform(TSTransform::new(egui::vec2(10.0, 20.0), 2.0));
        assert_eq!(geometry.zoom, 2.0);
        assert_eq!(geometry.to_global.translation, egui::vec2(10.0, 20.0));
        geometry.set_transform(TSTransform::new(egui::Vec2::ZERO, 0.001));
        assert_eq!(geometry.zoom, 0.05);
    }

    #[test]
    fn begin_frame_keeps_node_rects_but_drops_pin_rows() {
        let mut geometry = ViewGeometry::default();
        let node = NodeId(1);
        geometry.node_rects.insert(
            node,
            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0)),
        );
        geometry.input_pin_y.insert(InPinId { node, input: 0 }, 5.0);
        geometry.begin_frame();
        assert!(geometry.input_pin_y.is_empty());
        assert_eq!(geometry.node_rects.len(), 1);
        geometry.clear();
        assert!(geometry.node_rects.is_empty());
    }

    #[test]
    fn pin_style_builders_override_the_defaults() {
        let style = PinStyle::square().with_fill(Color32::RED);
        assert_eq!(style.shape, PinShape::Square);
        assert_eq!(style.fill, Color32::RED);
        assert_eq!(style.wire, pal::SOCKET);
    }

    /// The architecture rule: only the Snarl backend and the storage behind
    /// `NodeGraph` may name the widget crate.
    #[test]
    fn widget_crate_is_confined_to_backend_modules() {
        const ALLOWED: [&str; 2] = ["nodegraph.rs", "view/snarl.rs"];
        let needle = format!("{}_{}", "egui", "snarl");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

        let mut stack = vec![root.clone()];
        let mut offenders = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                let relative = path
                    .strip_prefix(&root)
                    .expect("inside src")
                    .to_string_lossy()
                    .replace('\\', "/");
                if ALLOWED.contains(&relative.as_str()) || relative == "view/mod.rs" {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read source");
                if text.contains(&needle) {
                    offenders.push(relative);
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these files name the widget crate directly: {offenders:?}"
        );
    }
}
