with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# Find the start_remove_bg_v2 function
idx = content.find("/// Start background removal using the new RemoveBg library (RMBG-1.4 logic).")
if idx >= 0:
    # Show context
    print(repr(content[idx:idx+500]))
