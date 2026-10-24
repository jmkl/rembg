with open("crates/compnode/src/nodes/image.rs", "rb") as f:
    content = f.read()

old = b"ui.columns(2, |cols| {\r\n        pick = cols[0].add(egui::Button::new(\"Replace\xe2\x80\xa6\").small()).clicked();\r\n        reload = cols[1].add(egui::Button::new(\"Reload\").small()).clicked();\r\n    });"

new = b"""ui.columns(2, |cols| {
        pick = cols[0].add(
            Button::new(\"Replace\")
                .accent(Accent::Green)
                .size(elegance::ButtonSize::Small)
                .full_width(),
        ).clicked();
        reload = cols[1].add(
            Button::new(\"Reload\")
                .accent(Accent::Red)
                .size(elegance::ButtonSize::Small)
                .full_width(),
        ).clicked();
    });"""

if old in content:
    content = content.replace(old, new)
    with open("crates/compnode/src/nodes/image.rs", "wb") as f:
        f.write(content)
    print("Replaced successfully")
else:
    print("Pattern not found")
