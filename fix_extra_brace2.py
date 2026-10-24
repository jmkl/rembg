with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# The function ends at position 15121 with "}\n\n"
# Then there's an extra "}\n" at position 15124
# Remove the extra closing brace
new_content = content[:15122] + content[15124:]

with open("crates/compnode/src/editor.rs", "w", encoding="utf-8") as f:
    f.write(content)

print("Fixed extra closing brace")
