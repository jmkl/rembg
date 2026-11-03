use eframe::egui::CursorIcon::PointingHand;
use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, vec2};

use crate::consts::{BTN_H, BTN_W, DANGER, FG, MUTED, TEXT};

pub struct Btn {}
impl Btn {
    /// `muted` renders the button in the muted color and disables clicks.
    pub fn click(
        ui: &mut egui::Ui,
        center: Pos2,
        id: &str,
        icon: &str,
        tip: &str,
        danger: bool,
        muted: bool,
    ) -> bool {
        let rect = Rect::from_center_size(center, vec2(BTN_W, BTN_H));
        let sense = if muted {
            Sense::hover()
        } else {
            Sense::click()
        };
        let resp = ui
            .interact(rect, ui.id().with(id), sense)
            .on_hover_cursor(PointingHand)
            .on_hover_text(tip);

        if muted {
            let p = ui.painter();
            let galley = p.layout_no_wrap(icon.to_owned(), FontId::proportional(14.0), DANGER);
            p.galley(
                rect.center() - galley.mesh_bounds.center().to_vec2(),
                galley,
                MUTED,
            );
            return false;
        }

        let down = resp.is_pointer_button_down_on();
        let (fill, fg) = if danger {
            match (resp.hovered(), down) {
                (_, true) => (DANGER.gamma_multiply(0.75), TEXT),
                (true, _) => (DANGER, TEXT),
                _ => (Color32::TRANSPARENT, MUTED),
            }
        } else {
            match (resp.hovered(), down) {
                (_, true) => (FG.gamma_multiply(0.30), TEXT),
                (true, _) => (FG.gamma_multiply(0.18), TEXT),
                _ => (Color32::TRANSPARENT, MUTED),
            }
        };

        let p = ui.painter();
        if fill != Color32::TRANSPARENT {
            p.rect_filled(rect, CornerRadius::same((0.0) as u8), fill);
        }

        // Nerd Font glyphs come from a fallback font. epaint advances them using the
        // primary font's width but paints the (often wider) fallback quad from the
        // left edge, so `Align2::CENTER_CENTER` centres the too-narrow galley box
        // instead of the visible glyph. Centre the ink bounds instead.
        let galley = p.layout_no_wrap(icon.to_owned(), FontId::proportional(14.0), fg);
        p.galley(
            rect.center() - galley.mesh_bounds.center().to_vec2(),
            galley,
            fg,
        );

        resp.clicked()
    }
}
