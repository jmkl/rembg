with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# Find the function
idx = content.find("/// Start background removal using the new RemoveBg library (RMBG-1.4 logic).")
if idx >= 0:
    # Find the end of the function
    brace_count = 0
    func_end = idx
    for i in range(idx, len(content)):
        if content[i] == "{":
            brace_count += 1
        elif content[i] == "}":
            brace_count -= 1
            if brace_count == 0:
                func_end = i + 1
                break
    func_text = content[idx:func_end]
    print(f"Function at {idx}-{func_end}")

    # Find the context before the function
    start = max(0, idx - 200)
    print("Context before:")
    print(repr(content[start:idx+100]))

# Now remove the function from its current location
content = content[:idx] + content[func_end:]

# Find the end of GraphEditor impl block
graph_start = content.find("impl GraphEditor {")
brace_count = 0
graph_end = -1
for i in range(graph_start, len(content)):
    if content[i] == "{":
        brace_count += 1
    elif content[i] == "}":
        brace_count -= 1
        if brace_count == 0:
            graph_end = i
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
# graph_end is the position of the closing brace
new_content = content[:graph_end] + "\n" + func_text.strip() + "\n" + content[graph_end:]

with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
    f.write(new_content)

print("Moved function to GraphEditor impl block")
