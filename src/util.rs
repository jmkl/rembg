//! Small helpers shared by the transport handlers: base64, content types and
//! percent-decoding.

use anyhow::{Result, anyhow};
use base64::Engine;

/// Decode a base64 string, also accepting a `data:<mime>;base64,` prefix.
pub fn b64_decode(value: &str) -> Result<Vec<u8>> {
    let payload = value.rsplit("base64,").next().unwrap_or(value);
    base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| anyhow!("invalid base64 image: {e}"))
}

/// Percent-decode a URL path (or a single path segment) into a filesystem path.
pub fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

/// Best-effort content type for a path, used when serving static files.
pub fn content_type_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".tif") || lower.ends_with(".tiff") {
        "image/tiff"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "text/javascript; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}

/// Image extensions accepted by the file browser.
pub fn is_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif", ".tif", ".tiff",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

pub struct MultipartPart {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

pub fn parse_multipart(content_type: &str, body: &[u8]) -> anyhow::Result<Vec<MultipartPart>> {
    let boundary = extract_boundary(content_type)
        .ok_or_else(|| anyhow::anyhow!("missing multipart boundary"))?;
    let boundary_bytes = format!("--{}", boundary).into_bytes();
    let delim = format!("\r\n--{}", boundary).into_bytes();

    let mut parts = Vec::new();
    // find first boundary
    let first = find_sub(body, &boundary_bytes, 0)
        .ok_or_else(|| anyhow::anyhow!("no multipart boundary found"))?;
    let mut cursor = first + boundary_bytes.len();

    loop {
        if body[cursor..].starts_with(b"\r\n") {
            cursor += 2;
        }
        if cursor >= body.len() {
            break;
        }
        if body[cursor..].starts_with(b"--") {
            break;
        }
        let next = find_sub(body, &delim, cursor)
            .ok_or_else(|| anyhow::anyhow!("unterminated multipart part"))?;
        let part_bytes = &body[cursor..next];
        let header_end = find_sub(part_bytes, b"\r\n\r\n", 0)
            .ok_or_else(|| anyhow::anyhow!("malformed multipart part"))?;
        let headers = &part_bytes[..header_end];
        let mut data = part_bytes[header_end + 4..].to_vec();
        if data.ends_with(b"\r\n") {
            data.truncate(data.len() - 2);
        }
        let headers_str = String::from_utf8_lossy(headers);
        let mut name = None;
        let mut filename = None;
        let mut ctype = None;
        for line in headers_str.lines() {
            let line = line.trim();
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-disposition:") {
                for seg in line.split(';') {
                    let seg = seg.trim();
                    let seg_lower = seg.to_ascii_lowercase();
                    if let Some(rest) = seg_lower.strip_prefix("name=") {
                        // recover original casing slice
                        let _ = rest;
                        name = Some(unquote(seg["name=".len()..].trim()));
                    } else if seg_lower.starts_with("filename=") {
                        filename = Some(unquote(seg["filename=".len()..].trim()));
                    }
                }
            } else if lower.starts_with("content-type:") {
                ctype = Some(line["content-type:".len()..].trim().to_string());
            }
        }
        let name = name.ok_or_else(|| anyhow::anyhow!("part missing name"))?;
        parts.push(MultipartPart {
            name,
            filename,
            content_type: ctype,
            data,
        });
        cursor = next + delim.len();
    }
    Ok(parts)
}

fn extract_boundary(ct: &str) -> Option<String> {
    for seg in ct.split(';') {
        let seg = seg.trim();
        let lower = seg.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("boundary=") {
            // use original seg for value (case preserved in boundary? use rest from seg)
            let _ = rest;
            let value = seg["boundary=".len()..].trim();
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multipart_roundtrip() {
        let boundary = "----abc123";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{{\"x\":1}}\r\n--{b}\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\nPNGDATA\r\n--{b}--\r\n",
            b = boundary
        );
        let ct = format!("multipart/form-data; boundary={}", boundary);
        let parts = parse_multipart(&ct, body.as_bytes()).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "manifest");
        assert_eq!(parts[0].data, b"{\"x\":1}");
        assert_eq!(parts[1].name, "f");
        assert_eq!(parts[1].filename.as_deref(), Some("a.png"));
        assert_eq!(parts[1].data, b"PNGDATA");
    }
}

fn find_sub(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start);
    }
    if start > haystack.len() {
        return None;
    }
    (start..=haystack.len().saturating_sub(needle.len()))
        .find(|&i| haystack[i..].starts_with(needle))
}
