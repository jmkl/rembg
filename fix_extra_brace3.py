with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# The function ends at position 15121 with "}\n\n"
# Then there's an extra "}\n" at position 15124
# The function should end with "}\n\n" (two closing braces: one for match, one for function)
# But it has "}\n\n}\n" (three closing braces)

# Fix: remove the extra closing brace at position 15124
# The content at 15121-15125 is "}\n\n}\n"
# It should be "}\n\n"

new_content = content[:15122] + content[15124:]

with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
    f.write(new_content)

print("Fixed")
