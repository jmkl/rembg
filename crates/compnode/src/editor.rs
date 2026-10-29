//! Snarl view implementation and node interaction.

use std::path::PathBuf;

use egui::{Align2, FontId, Frame, Stroke, vec2};
use egui_snarl::{
    InPin, InPinId, NodeId, OutPin, OutPinId, Snarl,
    ui::{
        AnyPins, BackgroundPattern, PinInfo, SnarlPin, SnarlStyle, SnarlViewer, get_selected_nodes,
    },
};

use crate::assets::AssetStore;
use crate::catalog;
use crate::graph::{connect_input, full_res_source_path, invalidate_downstream, invalidate_node};
use crate::memory::ImageMemory;
use crate::model::{Node, ViewerLayer};
use crate::style::controls::{MenuItem, muted};
use crate::style::{self, BODY_MARGIN, font, icons, pal};
use crate::widgets::socket;
use crate::worker::{FilterInput, Job, WorkerRequest};

pub(crate) struct GraphEditor {
    /// Current canvas scale, used to keep the dot grid a constant size.
    pub(crate) zoom: f32,
    /// Canvas-to-screen transform, used to place graph-menu nodes under the
    /// cursor in graph coordinates.
    pub(crate) to_global: egui::emath::TSTransform,
    pub(crate) assets: AssetStore,
    pub(crate) node_rects: std::collections::HashMap<egui_snarl::NodeId, egui::Rect>,
    /// Screen-space Y of each pin row, recorded while the node is drawn. Wires
    /// are straight lines between the node edges and these heights, which is
    /// all the cut gesture needs.
    input_pin_y: std::collections::HashMap<InPinId, f32>,
    output_pin_y: std::collections::HashMap<OutPinId, f32>,
    /// Active Ctrl+drag wire cut (Blender-style). Screen coordinates.
    cut_drag: Option<CutDrag>,
    jobs: std::sync::mpsc::Sender<WorkerRequest>,
    next_token: u64,
    /// ID for the Snarl widget's temporary view state. Replaced when the graph
    /// is loaded or cleared so pan/zoom and selection cannot leak across files.
    view_id: egui::Id,
    view_generation: u64,
    /// App-level node selection. egui-snarl 0.12 only updates its own selection
    /// for Shift/Ctrl-click and rectangle selection, so a normal click would
    /// otherwise leave Delete with nothing to act on.
    pub(crate) selected: Vec<NodeId>,
    /// Snarl's selection from the previous frame. Rectangle selection still
    /// comes from Snarl, so mirror it when it changes.
    snarl_selection: Vec<NodeId>,
    node_pressed: bool,
    /// Tracks recent full-resolution image use and caps CPU memory.
    image_memory: ImageMemory,
}

/// Path of a Ctrl+drag wire cut, in screen coordinates.
#[derive(Clone, Copy)]
struct CutDrag {
    start: egui::Pos2,
    last: egui::Pos2,
    clip: egui::Rect,
}

impl GraphEditor {
    pub(crate) fn new(jobs: std::sync::mpsc::Sender<WorkerRequest>) -> Self {
        Self {
            zoom: 1.0,
            to_global: egui::emath::TSTransform::IDENTITY,
            assets: AssetStore::default(),
            node_rects: std::collections::HashMap::new(),
            input_pin_y: std::collections::HashMap::new(),
            output_pin_y: std::collections::HashMap::new(),
            cut_drag: None,
            jobs,
            next_token: 1,
            view_id: egui::Id::new("snarl"),
            view_generation: 0,
            selected: Vec::new(),
            snarl_selection: Vec::new(),
            node_pressed: false,
            image_memory: ImageMemory::default(),
        }
    }

    pub(crate) fn view_id(&self) -> egui::Id {
        self.view_id
    }

    pub(crate) fn selected_nodes(&self) -> &[NodeId] {
        &self.selected
    }

    /// Drop all view state when the graph itself is replaced. egui-snarl keeps
    /// pan/zoom and selection in context storage keyed by the widget ID, so a
    /// new ID is the only way to guarantee they do not leak into a new file.
    pub(crate) fn reset_view(&mut self) {
        self.view_generation += 1;
        self.view_id = egui::Id::new(("snarl", self.view_generation));
        self.selected.clear();
        self.snarl_selection.clear();
        self.node_pressed = false;
        self.image_memory.clear();
        self.node_rects.clear();
        self.input_pin_y.clear();
        self.output_pin_y.clear();
        self.cut_drag = None;
    }

    /// Start a Blender-style wire cut when Ctrl+primary is pressed on empty
    /// graph background. Called before the Snarl widget so the started drag
    /// does not pan the canvas (`current_transform` suppresses that).
    pub(crate) fn begin_cut_drag(&mut self, ctx: &egui::Context, graph_rect: egui::Rect) {
        let Some(start) = ctx.input(|input| {
            (input.modifiers.command && input.pointer.primary_pressed())
                .then(|| input.pointer.interact_pos())
                .flatten()
        }) else {
            return;
        };
        if !graph_rect.contains(start) {
            return;
        }
        // Never start over a node (or its pins), so Ctrl+click on a node still
        // toggles its selection.
        if self
            .node_rects
            .values()
            .any(|rect| rect.expand(6.0).contains(start))
        {
            return;
        }
        self.cut_drag = Some(CutDrag {
            start,
            last: start,
            clip: graph_rect,
        });
    }

    /// Finish a Ctrl+drag wire cut. Runs after the Snarl widget has drawn, so
    /// this frame's pin heights are available. Every wire crossing the gesture
    /// is disconnected.
    pub(crate) fn update_cut_drag(&mut self, ctx: &egui::Context, snarl: &mut Snarl<Node>) {
        let Some(drag) = self.cut_drag.as_mut() else {
            return;
        };
        if let Some(pointer) = ctx.input(|input| input.pointer.latest_pos()) {
            drag.last = pointer;
        }

        let down = ctx.input(|input| input.pointer.primary_down());
        if down {
            // Cut preview: a thin bright line with a soft dark outline.
            // `Order::Middle` keeps it above the canvas but below popups.
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("snarl_cut_gesture"),
            ));
            let painter = painter.with_clip_rect(drag.clip);
            painter.line_segment(
                [drag.start, drag.last],
                Stroke::new(4.0, egui::Color32::from_black_alpha(120)),
            );
            painter.line_segment(
                [drag.start, drag.last],
                Stroke::new(1.5, egui::Color32::from_rgb(255, 90, 90)),
            );
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            ctx.request_repaint();
            return;
        }

        let (start, end) = (drag.start, drag.last);
        self.cut_drag = None;
        if start.distance(end) < 4.0 {
            return;
        }
        self.cut_wires(snarl, start, end);
        // The sidebar layer list and canvas need to reflect the removed wires.
        ctx.request_repaint();
    }

    /// Disconnect every wire whose straight line crosses `from`–`to`.
    fn cut_wires(&mut self, snarl: &mut Snarl<Node>, from: egui::Pos2, to: egui::Pos2) {
        let wires: Vec<(OutPinId, InPinId)> = snarl
            .wires()
            .filter(|(out, input)| {
                let (Some(out_rect), Some(in_rect)) = (
                    self.node_rects.get(&out.node),
                    self.node_rects.get(&input.node),
                ) else {
                    return false;
                };
                let (Some(out_y), Some(in_y)) =
                    (self.output_pin_y.get(out), self.input_pin_y.get(input))
                else {
                    return false;
                };
                segments_intersect(
                    from,
                    to,
                    egui::pos2(out_rect.right(), *out_y),
                    egui::pos2(in_rect.left(), *in_y),
                )
            })
            .collect();

        for (out, input) in wires {
            snarl.disconnect(out, input);
            invalidate_node(snarl, input.node);
        }
    }

    pub(crate) fn touch_image(&mut self, node: NodeId) {
        self.image_memory.touch(node);
    }

    pub(crate) fn node_at_screen(&self, pos: egui::Pos2) -> Option<NodeId> {
        self.node_rects
            .iter()
            .find(|(_, rect)| rect.contains(pos))
            .map(|(id, _)| *id)
    }

    pub(crate) fn trim_image_memory(&mut self, snarl: &mut Snarl<Node>) {
        self.image_memory.trim(snarl, &self.assets);
    }

    /// Remove specific nodes from the graph and invalidate affected downstream nodes.
    pub(crate) fn delete_nodes(&mut self, snarl: &mut Snarl<Node>, nodes: &[NodeId]) {
        let mut downstream = Vec::new();

        for node in nodes {
            self.selected.retain(|id| id != node);
            let Some(node_ref) = snarl.get_node(*node) else {
                continue;
            };
            let spec = catalog::spec(node_ref.kind());
            for output in 0..spec.outputs {
                for remote in snarl
                    .out_pin(OutPinId {
                        node: *node,
                        output,
                    })
                    .remotes
                {
                    if !nodes.contains(&remote.node) {
                        downstream.push(remote.node);
                    }
                }
            }
        }

        for node in nodes {
            if snarl.get_node(*node).is_some() {
                snarl.remove_node(*node);
            }
        }

        downstream.sort_by_key(|node| node.0);
        downstream.dedup();
        for node in downstream {
            if snarl.get_node(node).is_some() {
                invalidate_node(snarl, node);
            }
        }
        self.assets.retain_used(snarl);
    }

    /// Remove the app-level selection from the graph. Both Delete shortcuts and
    /// the Edit menu go through here so the selection cannot go stale.
    pub(crate) fn delete_selected(&mut self, snarl: &mut Snarl<Node>) {
        let removing = std::mem::take(&mut self.selected);
        self.delete_nodes(snarl, &removing);
    }

    /// Duplicate the selected nodes, offset down-right of the originals.
    /// Wiring between duplicated nodes is preserved, viewer/group layers are
    /// remapped to duplicated sources, and the duplicates become the new
    /// selection.
    pub(crate) fn duplicate_selected(&mut self, snarl: &mut Snarl<Node>) {
        use std::collections::HashMap;

        const OFFSET: f32 = 24.0;
        let selected = std::mem::take(&mut self.selected);
        let mut map: HashMap<NodeId, NodeId> = HashMap::new();

        for node in &selected {
            let Some(info) = snarl.get_node_info(*node) else {
                continue;
            };
            let mut value = info.value.clone();
            value.reset_transient_state();
            let new_id = snarl.insert_node(info.pos + egui::vec2(OFFSET, OFFSET), value);
            map.insert(*node, new_id);
        }

        // Keep the collapsed state of the sources.
        for (old, new) in &map {
            let open = snarl.get_node_info(*old).map_or(true, |src| src.open);
            if let Some(dst) = snarl.get_node_info_mut(*new) {
                dst.open = open;
            }
        }

        for (old, new) in &map {
            // Rewire links whose both ends were duplicated.
            let inputs = catalog::spec(snarl[*new].kind()).inputs;
            for input in 0..inputs {
                let remotes = snarl.in_pin(InPinId { node: *old, input }).remotes.clone();
                for remote in remotes {
                    let Some(&remote_new) = map.get(&remote.node) else {
                        continue;
                    };
                    snarl.connect(
                        OutPinId {
                            node: remote_new,
                            output: remote.output,
                        },
                        InPinId { node: *new, input },
                    );
                }
            }

            // Viewer and Group layers follow duplicated sources.
            if let Some(layers) = snarl[*new].layers_mut() {
                for layer in layers {
                    if let Some(mapped) = map.get(&layer.source) {
                        layer.source = *mapped;
                    }
                }
            }
        }

        self.selected = map.values().copied().collect();
        self.selected.sort_by_key(|node| node.0);
    }

    /// Finish bookkeeping after the Snarl widget has been rendered.
    ///
    /// Returns `true` when a primary click landed on empty graph space (which
    /// also clears the node selection), so the caller can dismiss other
    /// selections such as the sidebar layer list.
    pub(crate) fn finish_frame(
        &mut self,
        ctx: &egui::Context,
        snarl: &mut Snarl<Node>,
        canvas: &egui::Response,
    ) -> bool {
        self.update_cut_drag(ctx, snarl);

        self.selected
            .retain(|node| snarl.get_node_info(*node).is_some());

        // A Shift-drag rectangle is still implemented by egui-snarl. Adopt its
        // result on release, but never let it overwrite a click handled by
        // `update_selection`.
        let snarl_selection = get_selected_nodes(self.view_id, ctx);
        let pointer_released = ctx.input(|input| input.pointer.primary_released());
        if !self.node_pressed && pointer_released && snarl_selection != self.snarl_selection {
            self.selected.clone_from(&snarl_selection);
        }
        self.snarl_selection = snarl_selection;

        // Clicking empty canvas clears the selection unless the user is adding
        // to it with a modifier.
        let empty_clicked = !self.node_pressed
            && canvas.clicked_by(egui::PointerButton::Primary)
            && !ctx.input(|input| input.modifiers.shift || input.modifiers.command);
        if empty_clicked {
            self.selected.clear();
        }

        self.node_pressed = false;
        empty_clicked
    }

    fn update_selection(&mut self, node: NodeId, modifiers: egui::Modifiers) {
        if modifiers.shift {
            if !self.selected.contains(&node) {
                self.selected.push(node);
            }
        } else if modifiers.command {
            if let Some(pos) = self.selected.iter().position(|selected| *selected == node) {
                self.selected.remove(pos);
            } else {
                self.selected.push(node);
            }
        } else {
            self.selected.clear();
            self.selected.push(node);
        }
    }

    pub(crate) fn start_remove_bg(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        ctx: &egui::Context,
    ) {
        let path = match processing_input_path(snarl, node) {
            Ok(path) => path,
            Err(message) => {
                snarl[node].set_error(message);
                return;
            }
        };
        let image = image::open(&path)
            .map_err(|err| format!("Could not load {}: {err}", path.display()))
            .map(|img| img.to_rgba8());
        match image {
            Ok(input) => {
                let token = self.next_token;
                self.next_token += 1;
                let source_name = crate::graph::source_name(snarl, node);
                if self
                    .jobs
                    .send(WorkerRequest::Job(Job::RemoveBg {
                        token,
                        node,
                        input,
                        source_name,
                    }))
                    .is_ok()
                {
                    if let Some(process) = snarl[node].process_mut() {
                        process.job = Some(token);
                        process.preview.error = None;
                    }
                    invalidate_downstream(snarl, node);
                    ctx.request_repaint();
                } else {
                    snarl[node].set_error("Inference worker is not running");
                }
            }
            Err(message) => snarl[node].set_error(message),
        }
    }

    pub(crate) fn start_face_restore(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        ctx: &egui::Context,
    ) {
        let Some(data) = snarl[node].face_restore_node() else {
            return;
        };
        let (strength, only_center) = (data.strength, data.only_center);

        let path = match processing_input_path(snarl, node) {
            Ok(path) => path,
            Err(message) => {
                snarl[node].set_error(message);
                return;
            }
        };
        let image = image::open(&path)
            .map_err(|err| format!("Could not load {}: {err}", path.display()))
            .map(|img| img.to_rgba8());
        match image {
            Ok(input) => {
                let token = self.next_token;
                self.next_token += 1;
                let source_name = crate::graph::source_name(snarl, node);
                let job = Job::FaceRestore {
                    token,
                    node,
                    input,
                    strength,
                    only_center,
                    source_name,
                };
                if self.jobs.send(WorkerRequest::Job(job)).is_ok() {
                    if let Some(process) = snarl[node].process_mut() {
                        process.job = Some(token);
                        process.preview.error = None;
                    }
                    invalidate_downstream(snarl, node);
                    ctx.request_repaint();
                } else {
                    snarl[node].set_error("Inference worker is not running");
                }
            }
            Err(message) => snarl[node].set_error(message),
        }
    }

    /// Kick off a full-resolution Group/Blend render from on-disk source
    /// images. Returns `false` when the node is not a layer-compositing
    /// filter, has no renderable layers (e.g. a generator source without an
    /// output file) or the worker is gone.
    pub(crate) fn start_group_render(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        signature: u64,
        ctx: &egui::Context,
    ) -> bool {
        let Some(group) = snarl[node].group_node() else {
            return false;
        };
        let is_blend = group.is_blend;
        let canvas_vec = if group.canvas_size.x < 1.0 || group.canvas_size.y < 1.0 {
            let (w, h) = crate::eval::canvas_size(snarl);
            egui::vec2(w as f32, h as f32)
        } else {
            group.canvas_size
        };
        if canvas_vec.x < 1.0 || canvas_vec.y < 1.0 {
            return false;
        }
        let canvas = (canvas_vec.x as u32, canvas_vec.y as u32);

        // Blend keeps its base/overlay semantics: the bottom layer is the
        // base, layers above it use the node's mode and opacity.
        let (mode, opacity) = if is_blend {
            (
                crate::params::string(&group.params, "mode", "source-over").to_owned(),
                crate::params::f32(&group.params, "opacity", 100.0).clamp(0.0, 100.0) / 100.0,
            )
        } else {
            ("source-over".to_owned(), 1.0)
        };

        let mut layers = Vec::new();
        for layer in &group.layers {
            // Full-resolution render needs every layer backed by a file. A
            // filter layer must have rendered its own output first; otherwise
            // walking upstream would use the unfiltered image.
            let Some(path) = full_res_source_path(snarl, layer.source) else {
                return false;
            };
            layers.push(crate::worker::CompositeLayer {
                source: layer.source,
                kind: layer.kind,
                path,
                center: layer.center,
                size: layer.size,
                angle: layer.angle,
            });
        }
        if layers.is_empty() {
            return false;
        }

        let token = self.next_token;
        self.next_token += 1;
        let source_name = crate::graph::source_name(snarl, node);
        let job = Job::Composite {
            token,
            node,
            layers,
            canvas,
            mode,
            opacity,
            source_name,
        };
        if self.jobs.send(WorkerRequest::Job(job)).is_ok() {
            if let Some(group) = snarl[node].group_node_mut() {
                group.job = Some(token);
                group.render_state = None;
                group.rendered = Some(signature);
                group.preview.error = None;
            }
            ctx.request_repaint();
            true
        } else {
            false
        }
    }

    /// Kick off a full-resolution render for any filter kind.
    ///
    /// Group and Blend keep their layer-based composite path. Every other
    /// filter evaluates its inputs at full resolution on the worker thread.
    /// Returns `false` when an input is not ready yet (for example an upstream
    /// filter has not finished its own full-resolution render), so the caller
    /// can retry after the debounce window.
    pub(crate) fn start_filter_render(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        signature: u64,
        ctx: &egui::Context,
    ) -> bool {
        if snarl[node].kind().is_layer_kind() {
            return self.start_group_render(snarl, node, signature, ctx);
        }
        let Some(filter) = snarl[node].filter_node() else {
            return false;
        };
        let kind = filter.kind;
        let params = filter.params.clone();
        let sockets = catalog::spec(snarl[node].kind()).sockets.len();
        let canvas = crate::eval::canvas_size(snarl);

        let mut inputs: Vec<Vec<FilterInput>> = Vec::with_capacity(sockets);
        for input in 0..sockets {
            let remotes = snarl
                .in_pin(egui_snarl::InPinId { node, input })
                .remotes
                .clone();
            let mut socket = Vec::with_capacity(remotes.len());
            for remote in remotes {
                let Some(source) = filter_input(snarl, remote.node, &self.assets) else {
                    return false;
                };
                socket.push(source);
            }
            inputs.push(socket);
        }

        let token = self.next_token;
        self.next_token += 1;
        let source_name = crate::graph::source_name(snarl, node);
        let job = Job::Filter {
            token,
            node,
            kind,
            params,
            inputs,
            canvas,
            source_name,
        };
        if self.jobs.send(WorkerRequest::Job(job)).is_ok() {
            if let Some(filter) = snarl[node].filter_node_mut() {
                filter.job = Some(token);
                filter.render_state = None;
                filter.rendered = Some(signature);
                filter.preview.error = None;
            }
            ctx.request_repaint();
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    fn draw_background(
        &mut self,
        _background: Option<&BackgroundPattern>,
        viewport: &egui::Rect,
        _snarl_style: &SnarlStyle,
        _style: &egui::Style,
        painter: &egui::Painter,
        _snarl: &Snarl<Node>,
    ) {
        let zoom = self.zoom.max(0.05);
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

    /// Start background removal using the new RemoveBg library (RMBG-1.4 logic).
    pub(crate) fn start_remove_bg_v2(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        ctx: &egui::Context,
    ) {
        let path = match processing_input_path(snarl, node) {
            Ok(path) => path,
            Err(message) => {
                snarl[node].set_error(message);
                return;
            }
        };
        let image = image::open(&path)
            .map_err(|err| format!("Could not load {}: {err}", path.display()))
            .map(|img| img.to_rgba8());
        match image {
            Ok(input) => {
                let token = self.next_token;
                self.next_token += 1;
                let source_name = crate::graph::source_name(snarl, node);
                if self
                    .jobs
                    .send(WorkerRequest::Job(Job::RemoveBgV2 {
                        token,
                        node,
                        input,
                        source_name,
                    }))
                    .is_ok()
                {
                    if let Some(process) = snarl[node].process_mut() {
                        process.job = Some(token);
                        process.preview.error = None;
                    }
                    invalidate_downstream(snarl, node);
                    ctx.request_repaint();
                } else {
                    snarl[node].set_error("Inference worker is not running");
                }
            }
            Err(message) => snarl[node].set_error(message),
        }
    }
}

impl SnarlViewer<Node> for GraphEditor {
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
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        _snarl: &Snarl<Node>,
    ) -> Frame {
        style::node_frame(default, self.selected.contains(&node))
    }

    fn header_frame(
        &mut self,
        default: Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<Node>,
    ) -> Frame {
        style::node_header_frame(default, catalog::spec(snarl[node].kind()).accent)
    }

    fn show_header(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        const ROW_H: f32 = 22.0; // header content height
        const HIT: f32 = 20.0; // square hit area of each icon
        const PAD: f32 = 4.0; // margin from the node edge

        let open = snarl.get_node_info(node).is_none_or(|info| info.open);
        let spec = catalog::spec(snarl[node].kind());

        // The header spans the node: at least the body width, wider when the
        // node frame is.
        let width = ui.available_width().max(spec.body_width);
        ui.set_min_width(width);
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), egui::Sense::hover());

        let left = egui::Rect::from_min_size(
            egui::pos2(rect.left() + PAD, rect.center().y - HIT / 2.0),
            vec2(HIT, HIT),
        );
        let right = egui::Rect::from_min_size(
            egui::pos2(rect.right() - PAD - HIT, rect.center().y - HIT / 2.0),
            vec2(HIT, HIT),
        );
        let title = egui::Rect::from_min_max(
            egui::pos2(left.right(), rect.top()),
            egui::pos2(right.left(), rect.bottom()),
        );

        // Collapse chevron (left).
        let chevron = if open {
            icons::CHEVRON_DOWN
        } else {
            icons::CHEVRON_RIGHT
        };
        let response = ui.interact(left, ui.id().with(("chevron", node)), egui::Sense::click());
        ui.painter().text(
            left.center(),
            Align2::CENTER_CENTER,
            chevron,
            icons::font(13.0),
            pal::HEADER_TEXT,
        );
        if response
            .on_hover_text(if open { "Collapse" } else { "Expand" })
            .clicked()
        {
            snarl.open_node(node, !open);
        }

        // Title (centre). Match the sidebar layer list naming so the node a
        // layer comes from is identifiable at a glance: `{type} #{node id}`.
        ui.painter().text(
            title.center(),
            Align2::CENTER_CENTER,
            format!("{} #{}", spec.title, node.0),
            FontId::proportional(font::BUTTON),
            pal::HEADER_TEXT,
        );

        // Preview toggle (right).
        let enabled = snarl[node].preview_enabled();
        let glyph = if enabled { icons::EYE } else { icons::EYE_OFF };
        let response = ui.interact(right, ui.id().with(("eye", node)), egui::Sense::click());
        ui.painter().text(
            right.center(),
            Align2::CENTER_CENTER,
            glyph,
            icons::font(16.0),
            pal::HEADER_TEXT,
        );
        if response.clicked() {
            snarl[node].set_preview_enabled(!enabled);
        }
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        // Square pins for multi-input nodes (Group, Output) and nodes that receive layers
        let spec = catalog::spec(snarl[pin.id.node].kind());
        let is_multi = spec.multi_input;
        self.input_pin_y.insert(pin.id, pin_screen_y(ui));
        // Remove label - connectors should not have labels
        if is_multi {
            return PinInfo::square()
                .with_fill(pal::SOCKET)
                .with_stroke(Stroke::new(1.0, pal::SOCKET_STROKE))
                .with_wire_color(pal::SOCKET);
        }
        socket(ui, "")
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        self.output_pin_y.insert(pin.id, pin_screen_y(ui));
        socket(ui, "")
    }

    fn show_body(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        let spec = catalog::spec(snarl[node].kind());
        Frame::new().inner_margin(BODY_MARGIN).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(spec.body_width);
                (spec.body)(self, node, ui, snarl, spec.body_width);
            });
        });
    }

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: egui::Rect,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) {
        let to_global = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .unwrap_or(egui::emath::TSTransform::IDENTITY);
        self.node_rects.insert(node, to_global * rect);
        let (pressed, clicked) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_clicked(),
            )
        });
        if !(pressed || clicked) {
            return;
        }

        // Snarl draws nodes on its own transformed layer, but that layer has no
        // `Area`, so `Ui::rect_contains_pointer` fails its `layer_id_at` check.
        // Transform the node rect into screen space and compare the pointer
        // directly instead.
        let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
            return;
        };
        let to_global = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .unwrap_or(egui::emath::TSTransform::IDENTITY);
        if !(to_global * rect).contains(pointer) {
            return;
        }

        self.node_pressed = true;
        if pressed {
            let modifiers = ui.input(|input| input.modifiers);
            self.update_selection(node, modifiers);
        }
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        connect_input(snarl, from.id, to.id);

        // Keep Viewer layers in connection order. `refresh_viewer` fills in
        // textures and removes layers whose wire was disconnected.
        // Route is transparent: the layer belongs to the real source node.
        let source = crate::graph::resolve_route(snarl, from.id.node);
        // Extract all data from snarl immutably before borrowing viewer_node_mut().
        let spec = catalog::spec(snarl[source].kind());
        let source_name = spec.title.to_string();
        let source_path = snarl[source].output_path().map(|p| p.to_path_buf());
        let layer_exists = snarl[to.id.node]
            .viewer_node()
            .map(|v| v.layers.iter().any(|layer| layer.source == source))
            .unwrap_or(false);

        if let Some(viewer) = snarl[to.id.node].viewer_node_mut() {
            if !layer_exists {
                viewer.layers.push(ViewerLayer {
                    source,
                    kind: spec.kind,
                    texture: None,
                    source_name,
                    source_path,
                    center: egui::Vec2::ZERO,
                    size: egui::Vec2::ZERO,
                    angle: 0.0,
                });
            }
        }

        invalidate_node(snarl, to.id.node);
    }

    fn disconnect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        snarl.disconnect(from.id, to.id);
        // The input changed; drop the processed result and show the new input
        // (or no preview when nothing remains connected).
        invalidate_node(snarl, to.id.node);
    }

    fn drop_inputs(&mut self, pin: &InPin, snarl: &mut Snarl<Node>) {
        snarl.drop_inputs(pin.id);
        invalidate_node(snarl, pin.id.node);
    }

    fn drop_outputs(&mut self, pin: &OutPin, snarl: &mut Snarl<Node>) {
        let remotes = pin.remotes.clone();
        snarl.drop_outputs(pin.id);
        for remote in remotes {
            invalidate_node(snarl, remote.node);
        }
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
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        ui.label(muted("NODE", font::TINY));
        ui.add_space(2.0);
        if ui
            .add(MenuItem::new("Delete node").danger().icon(icons::TRASH))
            .clicked()
        {
            self.delete_nodes(snarl, &[node]);
            ui.close();
        }
    }

    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        _snarl: &mut Snarl<Node>,
    ) {
        // Pin heights are recorded again while this frame's nodes are drawn.
        self.input_pin_y.clear();
        self.output_pin_y.clear();

        // egui-snarl's `Scene` pans on primary drag for every button. While a
        // Ctrl+drag cut is active the canvas must stay put.
        if self.cut_drag.is_some() {
            to_global.translation = self.to_global.translation;
        }

        self.to_global = *to_global;
        self.zoom = to_global.scaling.max(0.05);
    }
}

/// Screen-space Y of the pin row currently being drawn.
fn pin_screen_y(ui: &egui::Ui) -> f32 {
    let to_global = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or(egui::emath::TSTransform::IDENTITY);
    (to_global * ui.max_rect().center()).y
}

/// Whether the line segments `a`–`b` and `c`–`d` cross or touch.
fn segments_intersect(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2, d: egui::Pos2) -> bool {
    fn cross(p: egui::Pos2, q: egui::Pos2, r: egui::Pos2) -> f32 {
        (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
    }
    fn on_segment(p: egui::Pos2, q: egui::Pos2, r: egui::Pos2) -> bool {
        q.x >= p.x.min(r.x) && q.x <= p.x.max(r.x) && q.y >= p.y.min(r.y) && q.y <= p.y.max(r.y)
    }

    let d1 = cross(c, d, a);
    let d2 = cross(c, d, b);
    let d3 = cross(a, b, c);
    let d4 = cross(a, b, d);

    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }

    (d1 == 0.0 && on_segment(c, a, d))
        || (d2 == 0.0 && on_segment(c, b, d))
        || (d3 == 0.0 && on_segment(a, c, b))
        || (d4 == 0.0 && on_segment(a, d, b))
}

/// Full-resolution file that a processing node should consume: the output of
/// its first upstream input.
///
/// Resolving from the input node (instead of walking from the processing node
/// itself) guarantees that a directly-upstream filter's rendered result is
/// used — never its low-res preview, the unfiltered image behind it, or the
/// processing node's own previous output.
fn processing_input_path(snarl: &Snarl<Node>, node: NodeId) -> Result<PathBuf, String> {
    match crate::graph::upstream(snarl, node) {
        Some(upstream) => full_res_source_path(snarl, upstream)
            .ok_or_else(|| "The input has no full-resolution output yet".to_owned()),
        None => Err("No input connected".to_owned()),
    }
}

/// Full-resolution input for a filter render.
///
/// Filters must have their own rendered output: using their preview texture or
/// walking past them would silently drop the filter effect. Image nodes follow
/// their upstream wire first (they are pass-through when connected) and fall
/// back to their file, asset bytes or decoded pixels.
fn filter_input(snarl: &Snarl<Node>, node: NodeId, assets: &AssetStore) -> Option<FilterInput> {
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(current) {
            return None;
        }
        // Route is a pass-through: its input is the real filter input.
        if snarl[current].is_route() {
            match crate::graph::upstream(snarl, current) {
                Some(next) => {
                    current = next;
                    continue;
                }
                None => return None,
            }
        }
        match &snarl[current] {
            Node::Filter(_) => {
                let path = snarl[current].output_path()?;
                return Some(FilterInput::Path(path.to_path_buf()));
            }
            Node::Image(image) => {
                if let Some(upstream) = crate::graph::upstream(snarl, current) {
                    current = upstream;
                    continue;
                }
                if !image.path.is_empty() && std::path::Path::new(&image.path).is_file() {
                    return Some(FilterInput::Path(PathBuf::from(&image.path)));
                }
                if let Some(asset_id) = image.asset_id.as_deref()
                    && let Some(asset) = assets.get(asset_id)
                {
                    return Some(FilterInput::Encoded(asset.encoded.clone()));
                }
                return image
                    .preview
                    .cpu
                    .as_ref()
                    .map(|cpu| FilterInput::Pixels(cpu.clone()));
            }
            _ => {
                if let Some(path) = snarl[current].output_path() {
                    return Some(FilterInput::Path(path.to_path_buf()));
                }
                if let Some(cpu) = snarl[current].cpu() {
                    return Some(FilterInput::Pixels(cpu.clone()));
                }
                if let Some(upstream) = crate::graph::upstream(snarl, current) {
                    current = upstream;
                    continue;
                }
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_segment_detects_crossings() {
        let a = egui::pos2(0.0, 0.0);
        let b = egui::pos2(100.0, 0.0);

        // Perpendicular crossing.
        assert!(segments_intersect(
            a,
            b,
            egui::pos2(50.0, -10.0),
            egui::pos2(50.0, 10.0),
        ));
        // Parallel, no crossing.
        assert!(!segments_intersect(
            a,
            b,
            egui::pos2(0.0, 10.0),
            egui::pos2(100.0, 10.0),
        ));
        // Crosses the infinite line but not the segment.
        assert!(!segments_intersect(
            a,
            b,
            egui::pos2(150.0, -10.0),
            egui::pos2(150.0, 10.0),
        ));
        // Touching an endpoint still cuts.
        assert!(segments_intersect(
            a,
            b,
            egui::pos2(100.0, 0.0),
            egui::pos2(100.0, 10.0),
        ));
    }

    #[test]
    fn cut_gesture_disconnects_only_crossed_wires() {
        let (jobs, _jobs_rx) = std::sync::mpsc::channel();
        let mut editor = GraphEditor::new(jobs);
        let mut snarl = Snarl::new();

        let image = snarl.insert_node(egui::pos2(0.0, 0.0), Node::image("C:/missing/a.png"));
        let filter = snarl.insert_node(egui::pos2(300.0, 0.0), Node::remove_bg());
        let from = OutPinId {
            node: image,
            output: 0,
        };
        let to = InPinId {
            node: filter,
            input: 0,
        };
        snarl.connect(from, to);

        let image_rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let filter_rect =
            egui::Rect::from_min_size(egui::pos2(300.0, 0.0), egui::vec2(200.0, 100.0));
        editor.node_rects.insert(image, image_rect);
        editor.node_rects.insert(filter, filter_rect);
        editor.output_pin_y.insert(from, 50.0);
        editor.input_pin_y.insert(to, 50.0);

        // A swipe below the wire leaves it connected.
        editor.cut_wires(&mut snarl, egui::pos2(250.0, 70.0), egui::pos2(250.0, 90.0));
        assert_eq!(snarl.wires().count(), 1);

        // A swipe across the wire cuts it.
        editor.cut_wires(&mut snarl, egui::pos2(250.0, 20.0), egui::pos2(250.0, 80.0));
        assert_eq!(snarl.wires().count(), 0);
    }
}
