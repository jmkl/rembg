mod image_canvas;

use eframe::egui;
use image_canvas::ImageCanvas;

struct App {
    canvas: ImageCanvas,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            // Drop the widget anywhere in your own UI:
            self.canvas.show(ui);
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("wgpu + egui image canvas")
            .with_inner_size([560.0, 940.0]),
        ..Default::default()
    };
    eframe::run_native(
        "wgpu-image-canvas",
        options,
        Box::new(|_cc| {
            Ok(Box::new(App {
                canvas: ImageCanvas::new(),
            }))
        }),
    )
}
