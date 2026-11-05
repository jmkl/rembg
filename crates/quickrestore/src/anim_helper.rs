use eframe::egui::{self, AsId};
use eframe::egui::{Id, Ui, emath::easing};

pub fn quart_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(4)
} // very smooth, premium feel

pub fn back_out(t: f32) -> f32 {
    // slight overshoot, "pop"
    let (c1, c3) = (1.70158_f32, 2.70158_f32);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

pub fn animate_me<F>(
    ui: &mut egui::Ui,
    anim_id: &str,
    target_value: f32,
    animation_time: f32,
    add_contents: F,
) where
    F: FnOnce(&mut egui::Ui, f32),
{
    let animated =
        ui.ctx()
            .animate_value_with_time(egui::Id::new(anim_id), target_value, animation_time);

    add_contents(ui, animated);
}

/// Cross-fades between two UIs depending on `show_b`.
/// Returns t: 0.0 = fully A, 1.0 = fully B (use it for width, height, etc.).
pub fn crossfade<S>(
    ui: &mut Ui,
    state: &mut S,
    id: impl AsId,
    show_b: bool,
    secs: f32,
    ease: fn(f32) -> f32,
    a: impl FnOnce(&mut Ui, &mut S),
    b: impl FnOnce(&mut Ui, &mut S),
) -> f32 {
    let t = ui
        .ctx()
        .animate_bool_with_time_and_easing(Id::new(id), show_b, secs, ease);

    if t < 1.0 {
        ui.scope(|ui| {
            ui.set_opacity(1.0 - t);
            // A fading-out side must not be clickable
            ui.add_enabled_ui(!show_b, |ui| a(ui, state));
        });
    }
    if t > 0.0 {
        ui.scope(|ui| {
            ui.set_opacity(t);
            ui.add_enabled_ui(show_b, |ui| b(ui, state));
        });
    }
    t
}
