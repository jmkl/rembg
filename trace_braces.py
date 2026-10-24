with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

# Trace brace balance
balance = 0
for i, ch in enumerate(content):
    if ch == "{":
        balance += 1
    elif ch == "}":
        balance -= 1
        if balance < 0:
            print(f"Extra closing brace at position {i}")
            break
    # Print balance at key positions
    if i in [43, 1854, 15501, 26000]:
        print(f"Position {i}: balance = {balance}")

print(f"Final balance: {balance}")
