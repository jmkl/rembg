with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

import re

# Find all impl blocks
for match in re.finditer(r"impl\s+\w+", content):
    print(f"Found at {match.start()}: {content[match.start():match.start()+50]}")

print()

# Count braces in the first 2000 chars (struct + impl GraphEditor)
substr = content[:2000]
print(f"First 2000 chars brace count: {{: {substr.count('{')}, }}: {substr.count('}')}")

# Find all impl blocks
for match in re.finditer(r"impl\s+[^{]+\{", content):
    print(f"Impl at {match.start()}: {content[match.start():match.start()+60]}")
