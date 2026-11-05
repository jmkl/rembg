//! compnode editor: an egui application around `compnode-ui`.
//!
//! ```text
//! cargo run --release -p compnode-editor -- [project.cnp]
//!     --bench [NODES]     scripted pan/zoom over a large graph, prints frame times
//! ```
//!
//! The window is a single canvas: the node graph, its minimap and the popups
//! that belong to it. There is no toolbar, no side panel and no image
//! processing — the graph is edited, not evaluated.
#![allow(deprecated)]

mod app;

use eframe::egui;
use std::path::PathBuf;

pub struct Args {
    pub bench: Option<usize>,
    pub project: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut a = Args {
        bench: None,
        project: None,
    };
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--bench" => {
                let n = it
                    .next_if(|v| v.parse::<usize>().is_ok())
                    .and_then(|v| v.parse().ok());
                a.bench = Some(n.unwrap_or(1000));
            }
            other if !other.starts_with("--") => a.project = Some(PathBuf::from(other)),
            other => eprintln!("ignoring unknown option {other}"),
        }
    }
    a
}

fn main() -> eframe::Result<()> {
    let args = parse_args();
    let options = eframe::NativeOptions {
        // Smooth (anti-aliased) edges: wires, node bodies and widgets are
        // submitted as raw meshes, which bypass the tessellator's feathering,
        // so the final edge AA comes from multisampling of the render target.
        multisampling: 4,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 900.0])
            .with_title("compnode"),
        ..Default::default()
    };
    eframe::run_native(
        "compnode",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, &args)))),
    )
}
