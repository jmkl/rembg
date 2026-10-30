use eframe::{
    egui::{self, FontData},
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
};

pub const FAMILIES: &[(&str, &str, &[u8])] = &[
    (
        "unisans-italic",
        "Unisans-italic",
        include_bytes!("../assets/fonts/Uni Sans Heavy Italic.otf"),
    ),
    (
        "unisans",
        "Unisans",
        include_bytes!("../assets/fonts/Uni Sans Heavy.otf"),
    ),
    (
        "anton",
        "Anton",
        include_bytes!("../assets/fonts/Anton-Regular.ttf"),
    ),
];

pub(crate) fn families() -> impl Iterator<Item = (&'static str, &'static str)> {
    FAMILIES.iter().map(|(id, name, _)| (*id, *name))
}

fn font_bytes(id: &str) -> &'static [u8] {
    FAMILIES
        .iter()
        .find(|(key, _, _)| *key == id)
        .map(|(_, _, bytes)| *bytes)
        .unwrap_or(FAMILIES[0].2)
}

pub fn install_fonts(ctx: &egui::Context) {
    ctx.add_font(FontInsert::new(
        "Inter",
        FontData::from_static(include_bytes!("../assets/fonts/Inter_18pt-Medium.ttf")),
        vec![InsertFontFamily {
            family: egui::FontFamily::Proportional,
            priority: FontPriority::Highest,
        }],
    ));
}
