with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# Find the end of GraphEditor impl block (before SnarlViewer impl)
snarl_pos = content.find("impl SnarlViewer<Node> for GraphEditor {")
if snarl_pos >= 0:
    func_text = """
    /// Start background removal using the new RemoveBg library (RMBG-1.4 logic).
    fn start_remove_bg_v2(
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
    new_content = content[:snarl_pos] + "\n" + func_text.strip() + "\n\n" + content[snarl_pos:]
    
    # Add missing closing braces at the end
    new_content = new_content + "\n}\n}\n"
    
    with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
        f.write(new_content)
    print("Fixed!")
else:
    print("SnarlViewer impl not found")
