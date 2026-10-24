with open("crates/compnode/src/editor.rs", "r", encoding="utf-8") as f:
    content = f.read()

idx = content.find("start_remove_bg_v2")
if idx >= 0:
    print("Found at:", idx)
    # Find the function end
    brace_count = 0
    found_start = False
    for i in range(idx, len(content)):
        if content[i] == "{":
            if not found_start:
                found_start = True
            brace_count += 1
        elif content[i] == "}":
            if found_start:
                brace_count -= 1
                if brace_count == 0:
                    print(f"Function ends at position {i}")
                    print(repr(content[i:i+200]))
                    break
        if i > idx + 5000:
            print("Search limit reached")
            break
else:
    print("Function not found")
