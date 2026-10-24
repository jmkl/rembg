with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

open_braces = content.count("{")
close_braces = content.count("}")
print("Open braces:", open_braces, "Close braces:", close_braces)
print("Diff:", open_braces - close_braces)

balance = 0
for i, ch in enumerate(content):
    if ch == "{":
        balance += 1
    elif ch == "}":
        balance -= 1
        if balance < 0:
            print("Extra closing brace at position", i)
            break

print("Final balance:", balance)
