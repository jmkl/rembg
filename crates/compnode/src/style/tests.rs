//! Headless UI smoke tests: run real egui frames through every widget in the
//! kit so font / layout / id problems show up as test failures, not as a
//! surprise at first launch.

use egui::{Event, PointerButton, Pos2, RawInput, Rect, vec2};

use super::controls::*;
use super::{icons, install_theme};

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0))),
        events,
        ..Default::default()
    }
}

/// Run one headless frame. Texture uploads are discarded (no renderer here);
/// egui panics if a frame's `TexturesDelta` is dropped without being cleared.
fn frame(ctx: &egui::Context, events: Vec<Event>, run: impl FnMut(&mut egui::Ui)) {
    ctx.run_ui(input(events), run).textures_delta.clear();
}

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    install_theme(&ctx);
    // First pass uploads the fonts registered by `install_theme`.
    frame(&ctx, vec![], |_| {});
    ctx
}

#[test]
fn icon_font_family_is_registered_and_lays_out() {
    let ctx = ctx();
    frame(&ctx, vec![], |ui| {
        let galley = ui.painter().layout_no_wrap(
            format!("{}{}{}", icons::EYE, icons::CHEVRON_DOWN, icons::X),
            icons::font(16.0),
            egui::Color32::WHITE,
        );
        assert!(galley.size().x > 8.0, "icon glyphs must have width");
    });
}

#[test]
fn every_widget_renders_without_panicking() {
    let ctx = ctx();
    for _ in 0..3 {
        frame(&ctx, vec![], |ui| {
            let mut on = true;
            let mut value = 0.4_f32;
            let mut choice = "b".to_owned();
            let mut color = egui::Color32::from_rgb(10, 120, 200);
            ui.add(Button::new("Primary"));
            ui.add(Button::new("Ok").success().small());
            ui.add(Button::new("Delete").danger().min_width(80.0));
            ui.add(Button::new("Ghost").outline().full_width());
            ui.add(Button::new("Working…").full_width().loading(true));
            ui.add(Button::icon(icons::SAVE).outline());
            ui.add(MenuItem::new("Save").shortcut("Ctrl+S").icon(icons::SAVE));
            ui.add(MenuItem::new("Quit").danger().icon(icons::X));
            ui.add(MenuItem::new("Off").enabled(false));
            ui.add(MenuItem::new("Fullscreen").checked(true));
            ui.add(Checkbox::new(&mut on, "Center face only"));
            ui.add(
                Slider::new(&mut value, 0.0..=1.0)
                    .label("Amount")
                    .suffix("%")
                    .step(0.1)
                    .decimals(1),
            );
            ui.add(
                Select::new("sel", &mut choice)
                    .label("Mode")
                    .options([("a".to_owned(), "Alpha"), ("b".to_owned(), "Beta")]),
            );
            ui.add(ColorField::new("col", &mut color).label("Tint"));
            error_line(ui, Some("something failed"));
            error_line(ui, None);
            window_button(ui, WindowGlyph::Minimize, "Minimize");
            window_button(ui, WindowGlyph::Maximize, "Maximize");
            window_button(ui, WindowGlyph::Restore, "Restore");
            window_button(ui, WindowGlyph::Close, "Close");
        });
    }
}

#[test]
fn slider_follows_the_pointer_and_snaps_to_step() {
    let ctx = ctx();
    let mut value = 0.0_f32;
    let mut rect = Rect::NOTHING;
    let mut run = |events: Vec<Event>, value: &mut f32, rect: &mut Rect| {
        frame(&ctx, events, |ui| {
            ui.set_width(200.0);
            let r = ui.add(Slider::new(value, 0.0..=100.0).step(10.0));
            *rect = r.rect;
        });
    };
    run(vec![], &mut value, &mut rect);
    // Press in the horizontal middle of the track.
    let target = Pos2::new(rect.center().x, rect.bottom() - 5.0);
    run(vec![Event::PointerMoved(target)], &mut value, &mut rect);
    run(
        vec![Event::PointerButton {
            pos: target,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }],
        &mut value,
        &mut rect,
    );
    assert!((value - 50.0).abs() < 1.0, "value was {value}");
    assert_eq!(value % 10.0, 0.0, "value must snap to the step");
}

#[test]
fn slider_double_click_resets_to_default() {
    let ctx = ctx();
    let mut value = 0.0_f32;
    let mut rect = Rect::NOTHING;
    let mut run = |events: Vec<Event>, value: &mut f32, rect: &mut Rect| {
        frame(&ctx, events, |ui| {
            ui.set_width(200.0);
            let r = ui.add(Slider::new(value, 0.0..=100.0).step(10.0).default(20.0));
            *rect = r.rect;
        });
    };
    run(vec![], &mut value, &mut rect);
    let at = Pos2::new(rect.left() + rect.width() * 0.75, rect.bottom() - 5.0);
    let click = |pressed: bool| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    // First click lands on the track: the value jumps to the pointer.
    run(
        vec![Event::PointerMoved(at), click(true)],
        &mut value,
        &mut rect,
    );
    run(vec![click(false)], &mut value, &mut rect);
    assert!(
        (value - 80.0).abs() < 1.0,
        "click should set the value, got {value}"
    );
    // The second click within the double-click delay restores the default.
    run(vec![click(true)], &mut value, &mut rect);
    run(vec![click(false)], &mut value, &mut rect);
    assert!(
        (value - 20.0).abs() < 1e-3,
        "double click must reset to the default, got {value}"
    );
}

#[test]
fn title_bar_and_settings_window_render() {
    let ctx = ctx();
    for _ in 0..2 {
        frame(&ctx, vec![], |ui| {
            let (_, h) = crate::title_bar::show(
                ui,
                "Untitled · autosaved",
                crate::style::pal::SUCCESS,
                1,
                false,
                false,
            );
            assert!(h > 0.0);
        });
    }
    let mut open = true;
    let mut settings = crate::settings::Settings::default();
    frame(&ctx, vec![], |ui| {
        let _ = crate::settings_window::show(ui.ctx(), &mut open, &mut settings);
    });
}

#[test]
fn menu_rows_fill_the_popup_width_without_blowing_it_up() {
    let ctx = ctx();
    let mut button = Rect::NOTHING;
    let mut rows: Vec<Rect> = Vec::new();
    let mut run = |events: Vec<Event>, button: &mut Rect, rows: &mut Vec<Rect>| {
        frame(&ctx, events, |ui| {
            let r = ui.menu_button("File", |ui| {
                rows.clear();
                rows.push(
                    ui.add(MenuItem::new("Save").shortcut("Ctrl+S").icon(icons::SAVE))
                        .rect,
                );
                rows.push(
                    ui.add(MenuItem::new("Save As…").shortcut("Ctrl+Shift+S"))
                        .rect,
                );
                rows.push(ui.add(MenuItem::new("Quit").danger()).rect);
            });
            *button = r.response.rect;
        });
    };
    run(vec![], &mut button, &mut rows);
    let at = button.center();
    for pressed in [true, false] {
        run(
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            &mut button,
            &mut rows,
        );
    }
    for _ in 0..3 {
        run(vec![], &mut button, &mut rows);
    }
    assert_eq!(rows.len(), 3, "popup should be open");
    let widths: Vec<f32> = rows.iter().map(|r| r.width()).collect();
    assert!(
        widths.iter().all(|w| (w - widths[0]).abs() < 0.5),
        "rows differ: {widths:?}"
    );
    assert!(
        widths[0] > 120.0 && widths[0] < 300.0,
        "unexpected popup width {widths:?}"
    );
}
