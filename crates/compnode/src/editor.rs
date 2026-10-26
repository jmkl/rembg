//! Snarl view implementation and node interaction.

use std::path::PathBuf;

use egui::{Align2, FontId, Frame, Stroke, vec2};
use egui_snarl::{
    InPin, NodeId, OutPin, OutPinId, Snarl,
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
    jobs: std::sync::mpsc::Sender<WorkerRequest>,
    next_token: u64,
    /// ID for the Snarl widget's temporary view state. Replaced when the graph
    /// is loaded or cleared so pan/zoom and selection cannot leak across files.
    view_id: egui::Id,
    view_generation: u64,
    /// App-level node selection. egui-snarl 0.12 only updates its own selection
    /// for Shift/Ctrl-click and rectangle selection, so a normal click would
    /// otherwise leave Delete with nothing to act on.
    selected: Vec<NodeId>,
    /// Snarl's selection from the previous frame. Rectangle selection still
    /// comes from Snarl, so mirror it when it changes.
    snarl_selection: Vec<NodeId>,
    node_pressed: bool,
    /// Tracks recent full-resolution image use and caps CPU memory.
    image_memory: ImageMemory,
}

impl GraphEditor {
    pub(crate) fn new(jobs: std::sync::mpsc::Sender<WorkerRequest>) -> Self {
        Self {
            zoom: 1.0,
            to_global: egui::emath::TSTransform::IDENTITY,
            assets: AssetStore::default(),
            node_rects: std::collections::HashMap::new(),
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

    /// Remove the app-level selection from the graph. Both Delete shortcuts and
    /// the Edit menu go through here so the selection cannot go stale.
    pub(crate) fn delete_selected(&mut self, snarl: &mut Snarl<Node>) {
        let removing = std::mem::take(&mut self.selected);
        let mut downstream = Vec::new();

        for node in &removing {
            if snarl.get_node_info(*node).is_none()
                || catalog::spec(snarl[*node].kind()).outputs == 0
            {
                continue;
            }
            for remote in snarl
                .out_pin(OutPinId {
                    node: *node,
                    output: 0,
                })
                .remotes
            {
                if !removing.contains(&remote.node) {
                    downstream.push(remote.node);
                }
            }
        }

        for node in removing {
            if snarl.get_node_info(node).is_some() {
                snarl.remove_node(node);
            }
        }

        downstream.sort_by_key(|node| node.0);
        downstream.dedup();
        for node in downstream {
            if snarl.get_node_info(node).is_some() {
                invalidate_node(snarl, node);
            }
        }
        self.assets.retain_used(snarl);
    }

    /// Finish bookkeeping after the Snarl widget has been rendered.
    pub(crate) fn finish_frame(
        &mut self,
        ctx: &egui::Context,
        snarl: &Snarl<Node>,
        canvas: &egui::Response,
    ) {
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
        if !self.node_pressed
            && canvas.clicked_by(egui::PointerButton::Primary)
            && !ctx.input(|input| input.modifiers.shift || input.modifiers.command)
        {
            self.selected.clear();
        }

        self.node_pressed = false;
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

    /// Kick off a full-resolution Group render from on-disk source images.
    /// Returns `false` when the node is not a Group, has no renderable layers
    /// (e.g. a generator source without an output file) or the worker is gone.
    pub(crate) fn start_group_render(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        signature: u64,
        ctx: &egui::Context,
    ) -> bool {
        let Some(filter) = snarl[node].filter_node() else {
            return false;
        };
        if filter.kind != crate::model::FilterKind::Group {
            return false;
        }
        let canvas_vec = filter.canvas_size;
        if canvas_vec.x < 1.0 || canvas_vec.y < 1.0 {
            return false;
        }
        let canvas = (canvas_vec.x as u32, canvas_vec.y as u32);

        let mut layers = Vec::new();
        for layer in &filter.layers {
            // Full-resolution render needs every layer backed by a file. A
            // filter layer must have rendered its own output first; otherwise
            // walking upstream would use the unfiltered image.
            let Some(path) = full_res_source_path(snarl, layer.source) else {
                return false;
            };
            layers.push(crate::worker::CompositeLayer {
                source: layer.source,
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

    /// Kick off a full-resolution render for any filter kind.
    ///
    /// Group nodes keep their layer-based composite path. Every other filter
    /// evaluates its inputs at full resolution on the worker thread. Returns
    /// `false` when an input is not ready yet (for example an upstream filter
    /// has not finished its own full-resolution render), so the caller can
    /// retry after the debounce window.
    pub(crate) fn start_filter_render(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        signature: u64,
        ctx: &egui::Context,
    ) -> bool {
        let Some(filter) = snarl[node].filter_node() else {
            return false;
        };
        let kind = filter.kind;
        if kind == crate::model::FilterKind::Group {
            return self.start_group_render(snarl, node, signature, ctx);
        }
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

    /// Blender-style dot grid, drawn in graph space with a screen-constant dot
    /// size and an adaptive spacing so the dot count stays bounded.
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
                painter.circle_filled(egui::pos2(x, y), radius, pal::SIDEBAR_LAYER_BG);
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
        let chevron = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
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

        // Title (centre).
        ui.painter().text(
            title.center(),
            Align2::CENTER_CENTER,
            spec.title,
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
        _pin: &OutPin,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
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
        // Keep Viewer layers in connection order. `refresh_viewer` fills in
        // textures and removes layers whose wire was disconnected.
        let source = from.id.node;
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
        pos: egui::Pos2,
        ui: &mut egui::Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<Node>,
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
            snarl.remove_node(node);
            ui.close();
        }
    }

    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        _snarl: &mut Snarl<Node>,
    ) {
        self.to_global = *to_global;
        self.zoom = to_global.scaling.max(0.05);
    }
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
