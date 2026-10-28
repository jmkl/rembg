// Quick ONNX model shape inspector — reads input/output metadata via ORT.
use anyhow::Result;
use ort::session::Session;
use std::env;

fn main() -> Result<()> {
    let path = env::args().nth(1).expect("Usage: inspect_onnx <model.onnx>");
    let session = Session::builder()?.commit_from_file(&path)?;

    println!("=== INPUTS ===");
    for input in session.inputs().iter() {
        println!("  name={:?}  dtype={:?}", input.name(), input.dtype());
    }
    Ok(())
}
