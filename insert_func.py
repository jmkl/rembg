with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# Find the end of impl GraphEditor block
start = content.find("impl GraphEditor {")
if start >= 0:
    brace_count = 0
    insert_pos = -1
    for i in range(start, len(content)):
        if content[i] == "{":
            brace_count += 1
        elif content[i] == "}":
            brace_count -= 1
            if brace_count == 0:
                insert_pos = i
                break

if insert_pos >= 0:
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
    new_content = content[:insert_pos] + "\n" + func_text + "\n" + content[insert_pos:]
    with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
        f.write(new_content)
    print("Inserted function at position", insert_pos)
else:
    print("Could not find closing brace")
