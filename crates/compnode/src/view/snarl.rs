//! egui-snarl backend for [`GraphView`].
//!
//! Together with `crate::nodegraph` this is the only code that names
//! `egui_snarl`. `SnarlAdapter` implements Snarl's `SnarlViewer` and forwards
//! every callback to the matching [`GraphEditor`] hook, translating Snarl's
//! ids and pin types into the crate's own. All behaviour lives in the editor.

use egui::emath::TSTransform;
use egui::{Color32, Frame, Margin, Shadow, Stroke, Vec2};
use egui_snarl::{
    InPin, NodeId as SnarlNodeId, OutPin, Snarl,
    ui::{
        AnyPins, BackgroundPattern, PinInfo, PinPlacement, SelectionStyle, SnarlPin, SnarlStyle,
        SnarlViewer, SnarlWidget, WireLayer, WireStyle, get_selected_nodes,
    },
};

use super::{GraphView, PinShape, PinStyle};
use crate::catalog;
use crate::editor::GraphEditor;
use crate::model::Node;
use crate::nodegraph::{NodeGraph, NodeId};
use crate::style::{self, controls, pal};

/// Draws the graph with egui-snarl.
pub(crate) struct SnarlView {
    style: SnarlStyle,
    /// ID for the Snarl widget's temporary view state. Replaced on reset so
    /// pan/zoom and selection cannot leak from one file into the next.
    id: egui::Id,
    generation: u64,
}

impl SnarlView {
    pub(crate) fn new() -> Self {
        Self {
            style: snarl_style(),
            id: egui::Id::new("snarl"),
            generation: 0,
        }
    }
}

impl GraphView for SnarlView {
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        graph: &mut NodeGraph<Node>,
        editor: &mut GraphEditor,
    ) -> egui::Response {
        SnarlWidget::new().id(self.id).style(self.style).show(
            graph.snarl_mut(),
            &mut SnarlAdapter { editor },
            ui,
        )
    }

    fn selection(&self, ctx: &egui::Context) -> Vec<NodeId> {
        get_selected_nodes(self.id, ctx)
            .into_iter()
            .map(NodeId::from)
            .collect()
    }

    /// egui-snarl keeps pan/zoom and selection in context storage keyed by the
    /// widget ID, so a new ID is the only way to guarantee they do not leak
    /// into a new file.
    fn reset(&mut self) {
        self.generation += 1;
        self.id = egui::Id::new(("snarl", self.generation));
    }

    fn generation(&self) -> u64 {
        self.generation
    }
}

/// Snarl hands its viewer a `&mut Snarl<Node>`; the editor works on
/// [`NodeGraph`]. Lend the content out as a `NodeGraph` for the duration of
/// `f`, then put it back. This is a move of a few headers, not a copy.
fn with_graph<T, R>(snarl: &mut Snarl<T>, f: impl FnOnce(&mut NodeGraph<T>) -> R) -> R {
    let mut graph = NodeGraph::from_snarl(std::mem::replace(snarl, Snarl::new()));
    let result = f(&mut graph);
    *snarl = graph.into_snarl();
    result
}

fn pin_info(style: PinStyle) -> PinInfo {
    let info = match style.shape {
        PinShape::Circle => PinInfo::circle(),
        PinShape::Square => PinInfo::square(),
    };
    info.with_fill(style.fill)
        .with_stroke(style.stroke)
        .with_wire_color(style.wire)
}

/// Screen-space Y of the pin row currently being drawn.
fn pin_screen_y(ui: &egui::Ui) -> f32 {
    let to_global = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or(TSTransform::IDENTITY);
    (to_global * ui.max_rect().center()).y
}

struct SnarlAdapter<'a> {
    editor: &'a mut GraphEditor,
}

impl SnarlViewer<Node> for SnarlAdapter<'_> {
    fn title(&mut self, node: &Node) -> String {
        catalog::spec(node.kind()).title.to_owned()
    }

    fn inputs(&mut self, node: &Node) -> usize {
        catalog::spec(node.kind()).inputs
    }

    fn outputs(&mut self, node: &Node) -> usize {
        catalog::spec(node.kind()).outputs
    }

    fn has_body(&mut self, _node: &Node) -> bool {
        true
    }

    fn node_frame(
        &mut self,
        default: Frame,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        _snarl: &Snarl<Node>,
    ) -> Frame {
        style::node_frame(default, self.editor.selected.contains(&NodeId::from(node)))
    }

    fn header_frame(
        &mut self,
        default: Frame,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<Node>,
    ) -> Frame {
        style::node_header_frame(default, catalog::spec(snarl[node].kind()).accent)
    }

    fn show_header(
        &mut self,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        with_graph(snarl, |graph| {
            self.editor.draw_node_header(node.into(), ui, graph);
        });
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        let screen_y = pin_screen_y(ui);
        let pin_style = with_graph(snarl, |graph| {
            self.editor
                .input_pin_style(pin.id.into(), screen_y, ui, graph)
        });
        pin_info(pin_style)
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        let screen_y = pin_screen_y(ui);
        let pin_style = with_graph(snarl, |graph| {
            self.editor
                .output_pin_style(pin.id.into(), screen_y, ui, graph)
        });
        pin_info(pin_style)
    }

    fn show_body(
        &mut self,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        with_graph(snarl, |graph| {
            self.editor.draw_node_body(node.into(), ui, graph);
        });
    }

    fn final_node_rect(
        &mut self,
        node: SnarlNodeId,
        rect: egui::Rect,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) {
        // Snarl lays nodes out on a transformed layer; the editor wants
        // screen space.
        let to_global = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .unwrap_or(TSTransform::IDENTITY);
        self.editor.node_laid_out(node.into(), to_global * rect, ui);
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        with_graph(snarl, |graph| {
            self.editor.on_connect(from.id.into(), to.id.into(), graph);
        });
    }

    fn disconnect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        with_graph(snarl, |graph| {
            self.editor
                .on_disconnect(from.id.into(), to.id.into(), graph);
        });
    }

    fn drop_inputs(&mut self, pin: &InPin, snarl: &mut Snarl<Node>) {
        with_graph(snarl, |graph| {
            self.editor.on_drop_inputs(pin.id.into(), graph);
        });
    }

    fn drop_outputs(&mut self, pin: &OutPin, snarl: &mut Snarl<Node>) {
        with_graph(snarl, |graph| {
            self.editor.on_drop_outputs(pin.id.into(), graph);
        });
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        // The stock Snarl graph menu uses `PopupCloseBehavior::CloseOnClick`,
        // which dismisses the popup as soon as the filter input is clicked.
        // `app` renders the menu with `CloseOnClickOutside` instead.
        false
    }

    fn has_node_menu(&mut self, _node: &Node) -> bool {
        true
    }

    fn has_dropped_wire_menu(&mut self, _src_pins: AnyPins, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_dropped_wire_menu(
        &mut self,
        _pos: egui::Pos2,
        _ui: &mut egui::Ui,
        _src_pins: AnyPins,
        _snarl: &mut Snarl<Node>,
    ) {
    }

    fn show_node_menu(
        &mut self,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        with_graph(snarl, |graph| {
            self.editor.draw_node_menu(node.into(), ui, graph);
        });
    }

    fn current_transform(&mut self, to_global: &mut TSTransform, _snarl: &mut Snarl<Node>) {
        self.editor.geometry.begin_frame();
        // While a wire cut is dragged the canvas must stay put.
        if self.editor.pan_locked() {
            to_global.translation = self.editor.geometry.to_global.translation;
        }
        self.editor.geometry.set_transform(*to_global);
    }
}

/// Configure egui-snarl's canvas, nodes, wires and selection.
fn snarl_style() -> SnarlStyle {
    let mut style = SnarlStyle::new();
    style.min_scale = Some(0.25);
    style.max_scale = Some(2.0);
    // The built-in triangle is replaced by our own chevron in
    // `GraphEditor::draw_node_header`.
    style.collapsible = Some(false);
    style.header_drag_space = Some(Vec2::ZERO);
    // NOTE: `crisp_magnified_text` multiplies the whole egui style by
    // `max_scale`, so enabling it makes nodes render at 2x/2.5x. Keep it off
    // and let the layer transform do the zooming.
    style.crisp_magnified_text = Some(false);

    // Wires: Blender-ish smooth curves behind the nodes.
    style.wire_style = Some(WireStyle::Line);
    style.wire_width = Some(2.0);
    style.wire_layer = Some(WireLayer::BehindNodes);

    // Sockets sit on the node border.
    style.pin_size = Some(10.0);
    style.pin_placement = Some(PinPlacement::Edge);
    style.pin_fill = Some(pal::SOCKET);
    style.pin_stroke = Some(Stroke::new(1.0, pal::SOCKET_STROKE));

    // Canvas: flat dark fill; no grid is drawn.
    style.bg_frame = Some(controls::canvas_frame());
    style.bg_pattern = Some(BackgroundPattern::NoPattern);

    // Frames are also overridden per node (see `node_frame` /
    // `node_header_frame`); these are the fallbacks.
    style.node_frame = Some(style::node_frame(
        Frame::new().shadow(Shadow {
            offset: [0, 5],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(50),
        }),
        false,
    ));
    style.header_frame =
        Some(style::node_header_frame(Frame::new(), pal::NODE).inner_margin(Margin::ZERO));

    // Rectangle selection uses the accent blue, like Blender's selection box.
    // NOTE: `select_stoke` is misspelled in egui-snarl's public API.
    style.select_stoke = Some(Stroke::new(2.0, pal::ACCENT));
    style.select_fill = Some(pal::ACCENT.gamma_multiply(0.08));
    // Node outlines are drawn through `GraphEditor::selected` instead, because egui-snarl
    // does not select a node on a normal click. Keep Snarl's own node
    // selection invisible so the two highlight sources never disagree.
    style.select_style = Some(SelectionStyle {
        fill: Color32::TRANSPARENT,
        stroke: Stroke::NONE,
        ..Default::default()
    });
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_gives_the_widget_a_new_id() {
        let mut view = SnarlView::new();
        let id = view.id;
        assert_eq!(view.generation(), 0);
        view.reset();
        assert_ne!(view.id, id);
        assert_eq!(view.generation(), 1);
    }

    #[test]
    fn with_graph_round_trips_the_snarl_content() {
        let mut snarl: Snarl<u8> = Snarl::new();
        let node = snarl.insert_node(egui::pos2(0.0, 0.0), 7);
        let seen = with_graph(&mut snarl, |graph| graph[NodeId::from(node)]);
        assert_eq!(seen, 7);
        assert_eq!(snarl[node], 7);
    }
}
