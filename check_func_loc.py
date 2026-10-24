with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

idx = content.find("start_remove_bg_v2")
if idx >= 0:
    # Show context
    start = max(0, idx - 200)
    print(repr(content[start:idx+300]))
