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
# Read the original SnarlViewer impl from the original file
with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    orig_content = f.read()

old_snarl_start = orig_content.find("impl SnarlViewer<Node> for GraphEditor {")
if old_snarl_start >= 0:
    brace_count = 0
    for i in range(old_snarl_start, len(orig_content)):
        if orig_content[i] == "{":
            brace_count += 1
        elif orig_content[i] == "}":
            brace_count -= 1
            if brace_count == 0:
                old_snarl_end = i + 1
                break
    snarl_text = orig_content[old_snarl_start:old_snarl_end]
    print(f"Original SnarlViewer impl length: {len(snarl_text)}")

# Remove the old SnarlViewer impl from new_content (it might be nested)
# Find and remove any nested impl SnarlViewer
new_snarl_pos = new_content.find("impl SnarlViewer<Node> for GraphEditor {")
if new_snarl_pos >= 0:
    brace_count = 0
    new_snarl_end = -1
    for i in range(new_snarl_pos, len(new_content)):
        if new_content[i] == "{":
            brace_count += 1
        elif new_content[i] == "}":
            brace_count -= 1
            if brace_count == 0:
                new_snarl_end = i + 1
                break
    if new_snarl_end > 0:
        # Remove the nested one
        new_content = new_content[:new_snarl_pos] + new_content[new_snarl_end:]
        print("Removed nested SnarlViewer impl")

# Add the SnarlViewer impl at the end
new_content = new_content + "\n" + snarl_text

with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
    f.write(new_content)
print("Done")
