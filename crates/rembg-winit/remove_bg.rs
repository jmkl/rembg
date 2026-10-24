use anyhow::{Context, Result};
use clap::Parser;
use rembg_core::{RemoveBg, config};

#[derive(Parser, Debug)]
#[command(
    name = "remove-bg",
    about = "RMBG-1.4 background removal (ONNX Runtime)"
)]
struct Args {
    #[arg(short, long, default_value = config::MODEL_RMBG)]
    model: String,
    /// Input image path
    #[arg(short, long, default_value = "test.jpg")]
    input: String,
    /// Output image path
    #[arg(short, long, default_value = "test_output.png")]
    output: String,
    /// Maximum input dimension (long edge), 0 = no limit
    #[arg(long, default_value_t = config::MAX_WIDTH)]
    max_width: u32,
    /// Use CPU even if CUDA is available
    #[arg(long)]
    cpu: bool,
    /// Warm up the session before processing
    #[arg(long)]
    warmup: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Loading model: {}", args.model);
    let rembg = RemoveBg::load_from(&args.model).context("Failed to load model")?;

    if args.warmup || config::WARMUP {
        println!("Warming up session...");
        rembg_warmup(&rembg).context("Warmup failed")?;
    }

    println!("Loading input image: {}", args.input);
    let input_img = image::open(&args.input).context("Failed to load input image")?;
    println!(
        "   Original size: {}x{}",
        input_img.width(),
        input_img.height()
    );

    // Apply max_width limit before processing (library already does this internally)
    let mut img = input_img;
    if args.max_width > 0 && img.width() > args.max_width {
        let h = (img.height() as f32 * args.max_width as f32 / img.width() as f32).round() as u32;
        img = img.resize_exact(args.max_width, h, image::imageops::FilterType::Lanczos3);
    }

    println!("Running inference...");
    let cutout = rembg.process_image(&img).context("Inference failed")?;

    println!("Saving output: {}", args.output);
    cutout
        .save(&args.output)
        .context("Failed to save output image")?;
    println!("Done! Output saved to {}", args.output);

    Ok(())
}

fn rembg_warmup(rembg: &RemoveBg) -> Result<()> {
    rembg.warmup()
}
