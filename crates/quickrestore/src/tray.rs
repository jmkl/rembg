//! Notification-area (system tray) icon and its menu.
//!
//! The tray lives on the winit message-pump thread, so menu clicks arrive on
//! that same thread. They are forwarded to the UI thread through a channel and
//! a repaint is requested, which also lets the app react while its window is
//! hidden.

use std::sync::mpsc;

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::consts::{BG, FG};

const TRAY_ID: &str = "quickrestore";
const MENU_SHOW: &str = "show";
const MENU_QUIT: &str = "quit";

/// What the user picked in the tray menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    Quit,
}

/// Keeps the tray icon alive for as long as the app is running.
pub struct Tray {
    // Dropping this removes the icon from the notification area.
    _icon: TrayIcon,
}

impl Tray {
    pub fn new(ctx: &egui::Context, tx: mpsc::Sender<TrayAction>) -> anyhow::Result<Self> {
        let show = MenuItem::with_id(MENU_SHOW, "Show", true, None);
        let quit = MenuItem::with_id(MENU_QUIT, "Quit", true, None);

        let menu = Menu::new();
        menu.append_items(&[&show, &quit])?;

        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                MENU_SHOW => TrayAction::Show,
                MENU_QUIT => TrayAction::Quit,
                _ => return,
            };
            let _ = tx.send(action);
            // Wake the event loop so `App::ui` runs and handles the action.
            ctx.request_repaint();
        }));

        let icon = TrayIconBuilder::new()
            .with_id(TRAY_ID)
            .with_tooltip("QuickRestore")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_icon(badge_icon())
            .build()?;

        Ok(Self { _icon: icon })
    }
}

/// A 32x32 "Q" badge drawn in the app palette, so the tray matches the HUD.
fn badge_icon() -> Icon {
    const SIZE: usize = 32;

    let center = SIZE as f32 / 2.0;
    let plate_half = center - 0.5;
    let plate_radius = 7.0;
    let ring_radius = 7.5;
    let stroke_half = 1.5;

    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5 - center;
            let py = y as f32 + 0.5 - center;

            let plate = coverage(round_box_distance(px, py, plate_half, plate_radius));
            let ring = coverage((px.hypot(py) - ring_radius).abs() - stroke_half);
            let tail = coverage(segment_distance(px, py, 4.5, 4.5, 10.5, 10.5) - stroke_half);

            let mut out = [0.0f32; 4];
            blend(&mut out, BG.to_array(), plate);
            blend(&mut out, FG.to_array(), ring);
            blend(&mut out, FG.to_array(), tail);

            rgba.extend_from_slice(&[to_u8(out[0]), to_u8(out[1]), to_u8(out[2]), to_u8(out[3])]);
        }
    }

    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).expect("32x32 rgba is a valid icon")
}

/// 1 px anti-aliased coverage from a signed distance.
fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

fn to_u8(v: f32) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

fn blend(dst: &mut [f32; 4], src: [u8; 4], coverage: f32) {
    let sa = (src[3] as f32 / 255.0) * coverage;
    let da = dst[3];
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }

    for i in 0..3 {
        let s = src[i] as f32 / 255.0;
        dst[i] = (s * sa + dst[i] * da * (1.0 - sa)) / out_a;
    }
    dst[3] = out_a;
}

fn round_box_distance(px: f32, py: f32, half: f32, radius: f32) -> f32 {
    let qx = px.abs() - (half - radius);
    let qy = py.abs() - (half - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

fn segment_distance(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len_sq = dx * dx + dy * dy;
    let t = if len_sq == 0.0 {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0)
    };
    ((px - (ax + t * dx)).powi(2) + (py - (ay + t * dy)).powi(2)).sqrt()
}
