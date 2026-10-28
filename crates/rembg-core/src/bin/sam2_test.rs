//! SAM 2.1 integration test / smoke-runner.
//!
//! Loads `sam2.1_hiera_small` (encoder + decoder), encodes the test image
//! once, then runs several decoder passes with different prompt strategies.
//! All outputs are written to `test_images/` next to the workspace root.
//!
//! Run from the workspace root:
//!   cargo run -p rembg-core --bin sam2_test

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use rembg_core::{PointLabel, Prompt, Sam2};

fn workspace_root() -> PathBuf {
    // Walk up from the manifest directory until we find the workspace Cargo.toml.
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            let content = std::fs::read_to_string(&candidate).unwrap_or_default();
            if content.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            // Fallback: use manifest dir
            return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        }
    }
}

fn main() -> Result<()> {
    let root = workspace_root();
    let images_dir = root.join("test_images");
    let input_path = images_dir.join("test.jpg");

    println!("=== SAM 2.1 hiera-small — integration test ===");
    println!("Workspace : {}", root.display());
    println!("Input     : {}", input_path.display());

    // -----------------------------------------------------------------------
    // 1. Load models
    // -----------------------------------------------------------------------
    let enc_path = root.join("models").join("sam2.1_hiera_small.encoder.onnx");
    let dec_path = root.join("models").join("sam2.1_hiera_small.decoder.onnx");

    println!("\n[1/4] Loading models …");
    println!("  encoder : {}", enc_path.display());
    println!("  decoder : {}", dec_path.display());

    let t = Instant::now();
    let sam = Sam2::load_from(&enc_path, &dec_path)
        .context("Failed to load SAM 2.1 models — check that both .onnx files exist")?;
    println!("  ✓ loaded in {:.2?}", t.elapsed());

    // -----------------------------------------------------------------------
    // 2. Open & inspect the test image
    // -----------------------------------------------------------------------
    println!("\n[2/4] Opening test image …");
    let img = image::open(&input_path)
        .with_context(|| format!("Cannot open {}", input_path.display()))?;
    let (w, h) = (img.width(), img.height());
    println!("  size : {w} × {h} px");

    // -----------------------------------------------------------------------
    // 3. Encode once, reuse for all decoder passes
    // -----------------------------------------------------------------------
    println!("\n[3/4] Encoding image …");
    let t = Instant::now();
    let embedding = sam.encode(&img).context("Encoder failed")?;
    println!("  ✓ encoded in {:.2?}", t.elapsed());
    let (emb_shape, hr0_shape, hr1_shape) = embedding.shapes();
    println!("  image_embed      shape: {emb_shape:?}");
    println!("  high_res_feats_0 shape: {hr0_shape:?}");
    println!("  high_res_feats_1 shape: {hr1_shape:?}");

    // -----------------------------------------------------------------------
    // 4. Decoder passes with different prompt strategies
    // -----------------------------------------------------------------------
    println!("\n[4/4] Decoding with various prompts …");

    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;

    let cases: &[(&str, Vec<Prompt>)] = &[
        // Single foreground click at the image centre
        (
            "sam2_center_point",
            vec![Prompt::Point {
                x: cx,
                y: cy,
                label: PointLabel::Foreground,
            }],
        ),
        // Tight bounding box covering the middle 60 % of the image
        (
            "sam2_center_box",
            vec![Prompt::Box {
                x1: w as f32 * 0.20,
                y1: h as f32 * 0.20,
                x2: w as f32 * 0.80,
                y2: h as f32 * 0.80,
            }],
        ),
        // Box + extra foreground point to reinforce the subject
        (
            "sam2_box_plus_point",
            vec![
                Prompt::Box {
                    x1: w as f32 * 0.20,
                    y1: h as f32 * 0.20,
                    x2: w as f32 * 0.80,
                    y2: h as f32 * 0.80,
                },
                Prompt::Point {
                    x: cx,
                    y: cy,
                    label: PointLabel::Foreground,
                },
            ],
        ),
        // Box + background click on the top-left corner to suppress background
        (
            "sam2_box_plus_bg",
            vec![
                Prompt::Box {
                    x1: w as f32 * 0.20,
                    y1: h as f32 * 0.20,
                    x2: w as f32 * 0.80,
                    y2: h as f32 * 0.80,
                },
                Prompt::Point {
                    x: w as f32 * 0.05,
                    y: h as f32 * 0.05,
                    label: PointLabel::Background,
                },
            ],
        ),
    ];

    for (name, prompts) in cases {
        let t = Instant::now();
        let mask = sam
            .decode(&embedding, prompts)
            .with_context(|| format!("Decoder failed for case '{name}'"))?;
        let elapsed = t.elapsed();

        // Save grayscale mask
        let mask_path = images_dir.join(format!("{name}_mask.png"));
        mask.save(&mask_path)
            .with_context(|| format!("Cannot save mask to {}", mask_path.display()))?;

        // Composite mask onto original image as alpha cutout
        let cutout = apply_mask(&img.to_rgba8(), &mask);
        let cutout_path = images_dir.join(format!("{name}_cutout.png"));
        cutout
            .save(&cutout_path)
            .with_context(|| format!("Cannot save cutout to {}", cutout_path.display()))?;

        println!(
            "  ✓ {name:<28} decoded in {elapsed:.2?}  →  {}  {}",
            mask_path.file_name().unwrap().to_string_lossy(),
            cutout_path.file_name().unwrap().to_string_lossy(),
        );
    }

    println!("\n=== All outputs written to {} ===", images_dir.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: apply a grayscale mask to the alpha channel of an RGBA image.
// ---------------------------------------------------------------------------
fn apply_mask(
    rgba: &image::RgbaImage,
    mask: &image::GrayImage,
) -> image::RgbaImage {
    assert_eq!(
        rgba.dimensions(),
        mask.dimensions(),
        "mask size must match image size"
    );
    let mut out = rgba.clone();
    for (p, m) in out.pixels_mut().zip(mask.pixels()) {
        p[3] = m[0];
    }
    out
}
