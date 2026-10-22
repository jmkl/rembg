use egui::Ui;
use egui_snarl::ui::SnarlViewer;
use egui_snarl::{NodeId, Snarl, ui::SnarlWidget};

struct Viewer;
impl SnarlViewer<String> for Viewer {
    fn title(&mut self, node: &String) -> String {
        node.clone()
    }
    fn inputs(&mut self, _node: &String) -> usize {
        0
    }
    fn outputs(&mut self, _node: &String) -> usize {
        1
    }
    fn has_body(&mut self, _node: &String) -> bool {
        true
    }
    fn show_input(
        &mut self,
        _pin: &egui_snarl::InPin,
        _ui: &mut Ui,
        _snarl: &mut Snarl<String>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        egui_snarl::ui::PinInfo::circle()
    }
    fn show_output(
        &mut self,
        _pin: &egui_snarl::OutPin,
        _ui: &mut Ui,
        _snarl: &mut Snarl<String>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        egui_snarl::ui::PinInfo::circle()
    }
    fn show_body(
        &mut self,
        _node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        _ui: &mut Ui,
        _snarl: &mut Snarl<String>,
    ) {
        _ui.label("blank node");
    }
}

struct BlankApp {
    snarl: Snarl<String>,
    viewer: Viewer,
}

impl BlankApp {
    fn new() -> Self {
        let mut snarl = Snarl::new();
        snarl.insert_node(egui::pos2(100.0, 100.0), "Node A".to_owned());
        snarl.insert_node(egui::pos2(300.0, 100.0), "Node B".to_owned());
        Self {
            snarl,
            viewer: Viewer,
        }
    }
}

impl eframe::App for BlankApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            SnarlWidget::new().show(&mut self.snarl, &mut self.viewer, ui);
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_title("Blank Snarl"),
        ..Default::default()
    };
    eframe::run_native(
        "Blank Snarl",
        options,
        Box::new(|_cc| Ok(Box::new(BlankApp::new()))),
    )
}
