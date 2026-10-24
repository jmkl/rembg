with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# Show context around the extra closing brace
pos = 15124
start = max(0, pos - 100)
end = min(len(content), pos + 100)
print(repr(content[start:end]))
