use eframe::egui::{
    self, Button, Color32, CornerRadius, RichText, Stroke, UiKind::CentralPanel, ViewportBuilder,
    Visuals, vec2,
};

struct QuickRestore {}
impl QuickRestore {
    fn new() -> Self {
        Self {}
    }
}
impl eframe::App for QuickRestore {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());

                let drag = ui.interact(
                    ui.max_rect(),
                    ui.id().with("drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    if ui
                        .add(Button::new(RichText::new("\u{f019f}").size(20.)))
                        .clicked()
                    {}
                    if ui
                        .add(Button::new(RichText::new("\u{f04b}").size(20.)))
                        .clicked()
                    {}
                });
            });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "QuickRestore",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_always_on_top()
                .with_inner_size(vec2(70., 30.))
                .with_decorations(false)
                .with_resizable(false)
                .with_has_shadow(false),
            ..Default::default()
        },
        Box::new(|cc| {
            let ctx = &cc.egui_ctx;
            ctx.set_visuals(Visuals::dark());
            ctx.style_mut_of(egui::Theme::Dark, |st| {
                let w = &mut st.visuals.widgets;
                w.inactive.weak_bg_fill = Color32::TRANSPARENT;
                w.inactive.corner_radius = CornerRadius::same(1);
                w.hovered.weak_bg_fill = Color32::TRANSPARENT;
                w.hovered.bg_stroke = Stroke::new(0., Color32::TRANSPARENT);
                w.active.bg_stroke = Stroke::new(0., Color32::from_rgb(30, 30, 30));
                w.active.weak_bg_fill = Color32::from_rgb(20, 20, 20);
            });
            let mut fonts = egui::FontDefinitions::default();
            egui_nerdfonts::add_to_fonts(&mut fonts, egui_nerdfonts::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(QuickRestore::new()))
        }),
    )
}
