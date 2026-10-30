use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Response, Sense, Ui, Widget, vec2,
};

pub struct Btn {
    ic: String,
    selected: bool,
    size: f32,
}
impl Btn {
    pub fn new(ic: impl Into<String>) -> Self {
        Self {
            ic: ic.into(),
            selected: false,
            size: 12.0,
        }
    }
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
    pub fn clicked(self, ui: &mut Ui, f: impl FnOnce()) -> Response {
        let response = ui.add(self);
        if response.clicked() {
            f();
        }
        response
    }
}

impl Widget for Btn {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());

        if ui.is_rect_visible(rect) {
            let v = *&ui.style().interact_selectable(&resp, self.selected);
            let mut fill = Color32::TRANSPARENT;
            if resp.hovered() {
                fill = Color32::from_rgb(10, 200, 100)
            }

            ui.painter().rect(
                rect.expand(v.expansion),
                CornerRadius::same(2),
                fill,
                v.bg_stroke,
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                self.ic,
                FontId::proportional(self.size),
                Color32::from_rgb(255, 255, 255),
            );
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}
