with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# Find the function
func_start = content.find("/// Start background removal using the new RemoveBg library (RMBG-1.4 logic).")
if func_start >= 0:
    brace_count = 0
    func_end = func_start
    for i in range(func_start, len(content)):
        if content[i] == "{":
            brace_count += 1
        elif content[i] == "}":
            brace_count -= 1
            if brace_count == 0:
                func_end = i + 1
                break
    func_text = content[func_start:func_end]
    print(f"Function at {func_start}-{func_end}")

# Find SnarlViewer impl
snarl_start = content.find("impl SnarlViewer<Node> for GraphEditor {")
if snarl_start >= 0:
    brace_count = 0
    for i in range(snarl_start, len(content)):
        if content[i] == "{":
            brace_count += 1
        elif content[i] == "}":
            brace_count -= 1
            if brace_count == 0:
                snarl_end = i + 1
                break
    snarl_text = content[snarl_start:snarl_end]
    print(f"SnarlViewer impl at {snarl_start}-{snarl_end}")

# Remove both from current locations
content = content[:func_start] + content[func_end:snarl_start] + content[snarl_end:]
print("Removed both")

# Find end of GraphEditor impl
graph_start = content.find("impl GraphEditor {")
brace_count = 0
graph_end = -1
for i in range(graph_start, len(content)):
    if content[i] == "{":
        brace_count += 1
    elif content[i] == "}":
        brace_count -= 1
        if brace_count == 0:
            graph_end = i + 1
            break

# Insert function before GraphEditor closing brace
func_text = """
    /// Start background removal using the new RemoveBg library (RMBG-1.4 logic).
    pub(crate) fn start_remove_bg_v2(
        &mut self,
        snarl: &mut Snarl<Node>,
        node: NodeId,
        ctx: &egui::Context,
    ) {
        let path = crate::graph::effective_output_path(snarl, node);
        let path = match path {
            Some(p) => p,
            None => {
                snarl[node].set_error("No input connected");
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
"""

# Insert function before GraphEditor closing brace
new_content = content[:graph_end] + "\n" + func_text.strip() + "\n" + content[graph_end:]

# Add SnarlViewer impl at the end
snarl_text = """
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

    fn show_header(
        &mut self,
        node: &Node,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> Frame {
        let spec = catalog::spec(node.kind());
        Frame::new().fill(spec.accent).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(spec.title).size(11.0).color(pal::HEADER_TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("−").clicked() {
                        // Collapse logic here
                    }
                });
            });
            ui.add_space(2.0);
        }
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

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        if matches!(snarl[pin.id.node], Node::Output(_)) {
            return PinInfo::square()
                .with_fill(pal::SOCKET)
                .with_stroke(Stroke::new(1.0, pal::SOCKET_STROKE))
                .with_wire_color(pal::SOCKET);
        }
        let spec = catalog::spec(snarl[pin.id.node].kind());
        let label = spec
            .sockets
            .get(pin.id.input)
            .map(|socket| socket.label)
            .unwrap_or("");
        socket(ui, label)
    }

    fn show_output(
        &mut self,
        _pin: &OutPin,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        socket(ui, "")
    }

    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        _snarl: &mut Snarl<Node>,
    ) {
        self.to_global = *to_global;
        self.zoom = to_global.scaling.max(0.05);
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
        let zoom = self.zoom.max(0.05)
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
                painter.circle_filled(egui::pos2(x, y), radius, pal::GRID);
                x += spacing;
            }
            y += spacing;
        }
    }
}

# Remove the old SnarlViewer impl from the file (it was nested)
# Find and remove the old one
old_snarl_start = content.find("impl SnarlViewer<Node> for GraphEditor {")
if old_snarl_start >= 0:
    # Find the old one (the nested one)
    brace_count = 0
    old_end = -1
    for i in range(old_snarl_start, len(content)):
        if content[i] == "{":
            brace_count += 1
        elif content[i] == "}":
            brace_count -= 1
            if brace_count == 0:
                old_end = i + 1
                break
    if old_end > 0:
        # Remove the old nested one
        pass

# For now, just append the SnarlViewer impl at the end
new_content = content + "\n" + snarl_impl_text

with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
    f.write(new_content)
print("Done")
