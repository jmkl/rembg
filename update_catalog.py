import pathlib

p = pathlib.Path("crates/compnode/src/catalog.rs")
s = p.read_text(encoding="utf-8")

# 1. Update import
s = s.replace("use crate::style::{BODY_W, VIEWER_W, pal};", "use crate::style::{NODE_W, pal};")

# 2. Replace all body_width: BODY_W with NODE_W
s = s.replace("body_width: BODY_W,", "body_width: NODE_W,")

# 3. Replace body_width: VIEWER_W with NODE_W
s = s.replace("body_width: VIEWER_W,", "body_width: NODE_W,")

# 4. Replace all socket labels with empty strings
# Pattern: label: "something",
s = s.replace('label: "Image",', 'label: "",')
s = s.replace('label: "Layers",', 'label: "",')
s = s.replace('label: "Base",', 'label: "",')
s = s.replace('label: "Blend",', 'label: "",')
s = s.replace('label: "Mask",', 'label: "",')
s = s.replace('label: "Image",', 'label: "",')

p.write_text(s, encoding="utf-8")
print("Done")
