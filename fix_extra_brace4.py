with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# The function ends with "}}\n" (two closing braces) but should end with "}\n"
# Find the position and fix
pos = 15122
# The content at pos is "}}\n\nimpl SnarlViewer"
# It should be "}\n\nimpl SnarlViewer"

new_content = content[:15120] + content[15122:]

with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
    f.write(new_content)

print("Fixed")
