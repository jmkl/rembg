//! Test YOLO11 + SAM2 pipeline to segment ONLY the person.
//!
//! Workflow:
//! 1. YOLO11 detects persons and returns bounding box(es).
//! 2. SAM2 encodes image once.
//! 3. SAM2 decodes with each person's bounding box as a prompt.
//! 4. Saves mask and cutout to test_images/.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use rembg_core::{Prompt, Sam2, Yolo};

fn workspace_root() -> PathBuf {
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
            return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        }
    }
}

fn main() -> Result<()> {
    let root = workspace_root();
    let images_dir = root.join("test_images");
    let input_path = images_dir.join("test.jpg");

    println!("=== YOLO11 + SAM 2.1 Person Segmentation Pipeline ===");
    println!("Workspace : {}", root.display());
    println!("Input     : {}", input_path.display());

    // 1. Load models
    println!("\n[1/4] Loading models …");
    let t = Instant::now();
    let yolo = Yolo::load().context("Failed to load YOLO model")?;
    println!("  ✓ YOLO11 loaded in {:.2?}", t.elapsed());

    let t = Instant::now();
    let sam = Sam2::load().context("Failed to load SAM2 models")?;
    println!("  ✓ SAM2 loaded in {:.2?}", t.elapsed());

    // 2. Open image
    println!("\n[2/4] Opening image …");
    let img = image::open(&input_path)
        .with_context(|| format!("Cannot open {}", input_path.display()))?;
    let (w, h) = (img.width(), img.height());
    println!("  size : {w} × {h} px");

    // 3. Detect persons with YOLO11
    println!("\n[3/4] Running YOLO11 person detector …");
    let t = Instant::now();
    let persons = yolo.detect_persons(&img, 0.35, 0.45)?;
    println!("  ✓ YOLO inference in {:.2?}", t.elapsed());
    println!("  Found {} person(s)", persons.len());

    if persons.is_empty() {
        return Err(anyhow!("No persons detected in {}", input_path.display()));
    }

    for (i, p) in persons.iter().enumerate() {
        println!(
            "  [{i}] score={:.2}% bbox=[x1={:.1}, y1={:.1}, x2={:.1}, y2={:.1}]",
            p.score * 100.0,
            p.bbox[0],
            p.bbox[1],
            p.bbox[2],
            p.bbox[3]
        );
    }

    // 4. SAM2 segmentation using YOLO person bounding boxes
    println!("\n[4/4] Running SAM2 segmentation with Person Box prompts …");
    let t = Instant::now();
    let embedding = sam.encode(&img).context("SAM2 encode failed")?;
    println!("  ✓ SAM2 encoded image in {:.2?}", t.elapsed());

    for (i, person) in persons.iter().enumerate() {
        let [x1, y1, x2, y2] = person.bbox;
        let prompt = Prompt::Box { x1, y1, x2, y2 };

        let t = Instant::now();
        let mask = sam.decode(&embedding, &[prompt])?;
        let decode_time = t.elapsed();

        let mask_file = format!("sam2_person_{i}_mask.png");
        let cutout_file = format!("sam2_person_{i}_cutout.png");

        let mask_path = images_dir.join(&mask_file);
        mask.save(&mask_path)?;

        let mut cutout = img.to_rgba8();
        for (p, m) in cutout.pixels_mut().zip(mask.pixels()) {
            p[3] = m[0];
        }
        let cutout_path = images_dir.join(&cutout_file);
        cutout.save(&cutout_path)?;

        println!(
            "  ✓ Person [{i}] decoded in {decode_time:.2?} -> {mask_file} & {cutout_file}"
        );
    }

    println!("\n=== Done! All results saved in {} ===", images_dir.display());
    Ok(())
}
