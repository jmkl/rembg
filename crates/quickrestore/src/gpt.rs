use arboard::Clipboard;
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use serde::Deserialize;
pub const INIT_JS: &str = include_str!("./script/gpt.js");

/// Messages the page sends back through `window.ipc.postMessage`.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GptEvent {
    /// The bridge script is installed (fires on every page load).
    Ready {
        url: String,
    },
    PromptSent {
        request_id: String,
    },
    ImageCopied {
        request_id: String,
        base64: String,
        mime_type: String,
        width: u32,
        height: u32,
    },
    BridgeError {
        request_id: String,
        error: String,
    },
}
/// `processPrompt(<base64|null>, <prompt>)`, with both args as JS literals.
/// Pass `None` for a text-only prompt.
pub fn process_prompt_js(png_base64: Option<&str>, prompt: &str) -> serde_json::Result<String> {
    Ok(format!(
        "processPrompt({},{})",
        serde_json::to_string(&png_base64)?,
        serde_json::to_string(prompt)?,
    ))
}

pub const ABORT_JS: &str = "abortPrompt()";
pub fn clipboard_image_base64() -> anyhow::Result<String> {
    let img = Clipboard::new()?.get_image()?; // RGBA8 pixels
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(
        &img.bytes,
        img.width as u32,
        img.height as u32,
        ExtendedColorType::Rgba8,
    )?;
    Ok(STANDARD.encode(png))
}
