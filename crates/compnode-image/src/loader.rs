//! Getting pixels in and out: a dependency-free PPM decoder, PNG/PPM
//! encoders, resolution fitting, thumbnails and a background loader.
//! Other formats (JPEG, WebP, decoding PNG, ...) belong to an `ImageLoader`
//! the host supplies, typically backed by the `image` crate.
use crate::buffer::ImageBuf;
use crate::cpu;
use compnode_core::color::srgb_to_linear;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Decodes a file into linear premultiplied pixels.
pub trait ImageLoader: Send + Sync {
    fn load(&self, path: &str) -> Result<ImageBuf, String>;
}

/// Loader for binary/ASCII PPM files (`P6`/`P3`).
pub struct PpmLoader;

impl ImageLoader for PpmLoader {
    fn load(&self, path: &str) -> Result<ImageBuf, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        decode_ppm(&bytes).map_err(|e| format!("{path}: {e}"))
    }
}

/// Loader backed by a closure (tests, hosts with their own decoder).
pub struct FnLoader<F: Fn(&str) -> Result<ImageBuf, String> + Send + Sync>(pub F);

impl<F: Fn(&str) -> Result<ImageBuf, String> + Send + Sync> ImageLoader for FnLoader<F> {
    fn load(&self, path: &str) -> Result<ImageBuf, String> {
        (self.0)(path)
    }
}

/// Serves images that were loaded elsewhere (for example by an
/// [`AsyncLoader`]); a path that is not there yet is an error, so evaluating
/// never touches the disk or blocks. Cloning shares the store.
#[derive(Clone, Default)]
pub struct StoreLoader {
    store: Arc<Mutex<compnode_core::hash::FxMap<String, Arc<ImageBuf>>>>,
    errors: Arc<Mutex<compnode_core::hash::FxMap<String, String>>>,
}

impl StoreLoader {
    pub fn new() -> StoreLoader {
        StoreLoader::default()
    }
    pub fn put(&self, path: &str, img: ImageBuf) {
        if let Ok(mut e) = self.errors.lock() {
            e.remove(path);
        }
        if let Ok(mut s) = self.store.lock() {
            s.insert(path.to_string(), Arc::new(img));
        }
    }
    /// Remember that loading `path` failed; `load` then reports `message`.
    pub fn fail(&self, path: &str, message: &str) {
        if let Ok(mut e) = self.errors.lock() {
            e.insert(path.to_string(), message.to_string());
        }
    }
    pub fn contains(&self, path: &str) -> bool {
        self.store.lock().is_ok_and(|s| s.contains_key(path))
    }
    pub fn remove(&self, path: &str) {
        if let Ok(mut s) = self.store.lock() {
            s.remove(path);
        }
        if let Ok(mut e) = self.errors.lock() {
            e.remove(path);
        }
    }
    pub fn clear(&self) {
        if let Ok(mut s) = self.store.lock() {
            s.clear();
        }
        if let Ok(mut e) = self.errors.lock() {
            e.clear();
        }
    }
}

impl ImageLoader for StoreLoader {
    fn load(&self, path: &str) -> Result<ImageBuf, String> {
        let img = self.store.lock().ok().and_then(|s| s.get(path).cloned());
        if let Some(a) = img {
            return Ok((*a).clone());
        }
        let failed = self.errors.lock().ok().and_then(|e| e.get(path).cloned());
        Err(failed.unwrap_or_else(|| format!("loading {path}...")))
    }
}

struct Tokens<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Tokens<'a> {
    fn skip_ws_and_comments(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'#' => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                c if c.is_ascii_whitespace() => self.i += 1,
                _ => break,
            }
        }
    }
    fn token(&mut self) -> Option<&'a str> {
        self.skip_ws_and_comments();
        let start = self.i;
        while self.i < self.b.len()
            && !self.b[self.i].is_ascii_whitespace()
            && self.b[self.i] != b'#'
        {
            self.i += 1;
        }
        if start == self.i {
            None
        } else {
            std::str::from_utf8(&self.b[start..self.i]).ok()
        }
    }
    fn number(&mut self, what: &str) -> Result<u32, String> {
        self.token()
            .ok_or_else(|| format!("missing {what}"))?
            .parse::<u32>()
            .map_err(|_| format!("bad {what}"))
    }
}

/// Decode a PPM (`P6` binary or `P3` ASCII, 8 or 16 bit) as opaque sRGB.
pub fn decode_ppm(bytes: &[u8]) -> Result<ImageBuf, String> {
    let mut t = Tokens { b: bytes, i: 0 };
    let magic = t.token().ok_or("empty file")?;
    let binary = match magic {
        "P6" => true,
        "P3" => false,
        other => return Err(format!("unsupported format {other:?} (only PPM P3/P6)")),
    };
    let w = t.number("width")?;
    let h = t.number("height")?;
    let maxval = t.number("maxval")?;
    if w == 0 || h == 0 || w > 32768 || h > 32768 {
        return Err("bad image size".into());
    }
    if maxval == 0 || maxval > 65535 {
        return Err("bad maxval".into());
    }
    let n = w as usize * h as usize * 3;
    let mut samples: Vec<u32> = Vec::with_capacity(n);
    if binary {
        t.i += 1; // the single whitespace byte after maxval
        let wide = maxval > 255;
        let need = n * if wide { 2 } else { 1 };
        let data = bytes.get(t.i..t.i + need).ok_or("truncated pixel data")?;
        if wide {
            samples.extend(
                data.chunks(2)
                    .map(|c| (u32::from(c[0]) << 8) | u32::from(c[1])),
            );
        } else {
            samples.extend(data.iter().map(|v| u32::from(*v)));
        }
    } else {
        for _ in 0..n {
            samples.push(t.number("sample")?);
        }
    }
    let mv = maxval as f32;
    let data = samples
        .chunks(3)
        .map(|p| {
            let c = |v: u32| srgb_to_linear((v as f32 / mv).min(1.0));
            [c(p[0]), c(p[1]), c(p[2]), 1.0]
        })
        .collect();
    Ok(ImageBuf {
        width: w,
        height: h,
        data,
    })
}

/// Binary PPM (`P6`), alpha dropped (color over black).
pub fn encode_ppm(img: &ImageBuf) -> Vec<u8> {
    let mut out = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
    let enc =
        |l: f32| (compnode_core::color::linear_to_srgb(l.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    for p in &img.data {
        out.extend_from_slice(&[enc(p[0]), enc(p[1]), enc(p[2])]);
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, slot) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    let mut c = 0xFFFF_FFFF_u32;
    for &b in data {
        c = table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// 8-bit RGBA PNG (straight alpha, sRGB). Uses stored (uncompressed)
/// deflate blocks, so files are large but valid and need no dependency.
pub fn encode_png(img: &ImageBuf) -> Vec<u8> {
    let rgba = img.to_srgb8_rgba();
    let stride = img.width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * img.height as usize);
    for row in rgba.chunks(stride.max(1)) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    if raw.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    let blocks = raw.chunks(65535).count();
    for (i, block) in raw.chunks(65535).enumerate() {
        z.push(u8::from(i + 1 == blocks));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&img.width.to_be_bytes());
    ihdr.extend_from_slice(&img.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportFormat {
    Png,
    Ppm,
    Jpeg,
    WebP,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Ppm => "ppm",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::WebP => "webp",
        }
    }
}

/// Write `img` to `path`. PNG and PPM are built in; JPEG (quality 90) and
/// lossless WebP need the `image-io` feature and report an error without it.
pub fn export(img: &ImageBuf, path: &str, format: ExportFormat) -> Result<(), String> {
    let bytes = match format {
        ExportFormat::Png => encode_png(img),
        ExportFormat::Ppm => encode_ppm(img),
        ExportFormat::Jpeg | ExportFormat::WebP => {
            #[cfg(feature = "image-io")]
            return crate::imageio::export_with_image_crate(img, path, format, 90);
            #[cfg(not(feature = "image-io"))]
            return Err(format!(
                "{} export is not available in this build (enable the `image-io` feature)",
                format.extension()
            ));
        }
    };
    std::fs::write(path, bytes).map_err(|e| format!("{path}: {e}"))
}

/// Shrink `img` to fit in `max_w x max_h`, keeping the aspect ratio. Images
/// that already fit are returned unchanged (never enlarged).
pub fn downscale_to_fit(img: ImageBuf, max_w: u32, max_h: u32) -> ImageBuf {
    let scale = (max_w as f32 / img.width as f32).min(max_h as f32 / img.height as f32);
    if scale >= 1.0 {
        return img;
    }
    let w = ((img.width as f32 * scale).round() as u32).max(1);
    let h = ((img.height as f32 * scale).round() as u32).max(1);
    cpu::resize(&img, w, h)
}

/// Small preview with the longer side at most `max_dim`.
pub fn thumbnail(img: &ImageBuf, max_dim: u32) -> ImageBuf {
    downscale_to_fit(img.clone(), max_dim, max_dim)
}

pub struct LoadResult {
    pub id: u64,
    pub path: String,
    pub result: Result<ImageBuf, String>,
}

/// Decodes files on worker threads so the UI never blocks. Results are
/// downscaled to the canvas size when `fit` is set.
pub struct AsyncLoader {
    tx: Option<Sender<(u64, String)>>,
    rx: Receiver<LoadResult>,
    workers: Vec<JoinHandle<()>>,
}

impl AsyncLoader {
    pub fn new(
        loader: Arc<dyn ImageLoader>,
        threads: usize,
        fit: Option<(u32, u32)>,
    ) -> AsyncLoader {
        let (tx, jobs) = channel::<(u64, String)>();
        let (done_tx, rx) = channel::<LoadResult>();
        let jobs = Arc::new(Mutex::new(jobs));
        let mut workers = Vec::new();
        for _ in 0..threads.max(1) {
            let (jobs, done, loader) = (Arc::clone(&jobs), done_tx.clone(), Arc::clone(&loader));
            workers.push(std::thread::spawn(move || {
                loop {
                    let job = {
                        let Ok(guard) = jobs.lock() else { break };
                        let received = guard.recv();
                        drop(guard);
                        received
                    };
                    let Ok((id, path)) = job else { break };
                    let result = loader.load(&path).map(|img| match fit {
                        Some((w, h)) => downscale_to_fit(img, w, h),
                        None => img,
                    });
                    if done.send(LoadResult { id, path, result }).is_err() {
                        break;
                    }
                }
            }));
        }
        AsyncLoader {
            tx: Some(tx),
            rx,
            workers,
        }
    }

    /// Queue a file. Returns immediately.
    pub fn submit(&self, id: u64, path: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send((id, path.to_string()));
        }
    }

    /// Finished loads so far (never blocks).
    pub fn poll(&self) -> Vec<LoadResult> {
        self.rx.try_iter().collect()
    }

    /// Wait for one result (tests and shutdown paths).
    pub fn wait(&self, timeout: Duration) -> Option<LoadResult> {
        self.rx.recv_timeout(timeout).ok()
    }
}

impl Drop for AsyncLoader {
    fn drop(&mut self) {
        self.tx.take(); // closing the queue ends the workers
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_core::Color;

    #[test]
    fn ppm_p6_p3_comments_and_errors() {
        let mut p6 = b"P6\n# a comment\n2 1\n255\n".to_vec();
        p6.extend_from_slice(&[255, 0, 0, 0, 0, 255]);
        let img = decode_ppm(&p6).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.get(0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(img.get(1, 0), [0.0, 0.0, 1.0, 1.0]);
        let p3 = b"P3 1 1 100 50 100 0";
        let img = decode_ppm(p3).unwrap();
        let p = img.get(0, 0);
        assert!((p[0] - srgb_to_linear(0.5)).abs() < 1e-6 && p[1] == 1.0 && p[2] == 0.0);
        // 16 bit
        let mut wide = b"P6 1 1 65535\n".to_vec();
        wide.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00, 0x80, 0x00]);
        let img = decode_ppm(&wide).unwrap();
        assert!(
            (img.get(0, 0)[0] - 1.0).abs() < 1e-6
                && (img.get(0, 0)[2] - srgb_to_linear(32768.0 / 65535.0)).abs() < 1e-5
        );
        assert!(decode_ppm(b"P5 1 1 255 x").is_err());
        assert!(
            decode_ppm(b"P6 2 2 255\nabc")
                .unwrap_err()
                .contains("truncated")
        );
        assert!(decode_ppm(b"").is_err());
        assert!(decode_ppm(b"P6 0 1 255\n").is_err());
    }

    #[test]
    fn ppm_roundtrip_through_encoder() {
        let img = ImageBuf::from_fn(3, 2, |x, y| {
            let c = Color::from_srgb8(x as u8 * 80, y as u8 * 100, 200, 255);
            [c.r, c.g, c.b, 1.0]
        });
        let back = decode_ppm(&encode_ppm(&img)).unwrap();
        assert!(img.max_abs_diff(&back).unwrap() < 1e-5);
    }

    fn parse_png(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        let mut i = 8;
        let mut out = Vec::new();
        while i < bytes.len() {
            let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = bytes[i + 4..i + 8].try_into().unwrap();
            let data = bytes[i + 8..i + 8 + len].to_vec();
            let crc = u32::from_be_bytes(bytes[i + 8 + len..i + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(&bytes[i + 4..i + 8 + len]), "crc of {kind:?}");
            out.push((kind, data));
            i += 12 + len;
        }
        out
    }

    fn inflate_stored(z: &[u8]) -> Vec<u8> {
        assert_eq!(&z[..2], &[0x78, 0x01]);
        let mut i = 2;
        let mut out = Vec::new();
        loop {
            let final_block = z[i] & 1 == 1;
            let len = u16::from_le_bytes([z[i + 1], z[i + 2]]);
            let nlen = u16::from_le_bytes([z[i + 3], z[i + 4]]);
            assert_eq!(len, !nlen);
            out.extend_from_slice(&z[i + 5..i + 5 + len as usize]);
            i += 5 + len as usize;
            if final_block {
                break;
            }
        }
        assert_eq!(
            u32::from_be_bytes(z[i..i + 4].try_into().unwrap()),
            adler32(&out)
        );
        out
    }

    #[test]
    fn png_is_structurally_valid_and_carries_the_pixels() {
        let img = ImageBuf::from_fn(3, 2, |x, y| {
            let a = 0.25 + 0.25 * x as f32;
            let c = Color::new(x as f32 * 0.4, y as f32 * 0.5, 1.0, a).premultiplied();
            [c.r, c.g, c.b, c.a]
        });
        let png = encode_png(&img);
        let chunks = parse_png(&png);
        let kinds: Vec<&[u8; 4]> = chunks.iter().map(|c| &c.0).collect();
        assert_eq!(kinds, vec![b"IHDR", b"IDAT", b"IEND"]);
        let ihdr = &chunks[0].1;
        assert_eq!(u32::from_be_bytes(ihdr[0..4].try_into().unwrap()), 3);
        assert_eq!(u32::from_be_bytes(ihdr[4..8].try_into().unwrap()), 2);
        assert_eq!(&ihdr[8..], &[8, 6, 0, 0, 0]);
        let raw = inflate_stored(&chunks[1].1);
        let rgba = img.to_srgb8_rgba();
        assert_eq!(raw.len(), 2 * (1 + 12));
        for (y, row) in raw.chunks(13).enumerate() {
            assert_eq!(row[0], 0);
            assert_eq!(&row[1..], &rgba[y * 12..(y + 1) * 12]);
        }
    }

    #[test]
    fn png_with_more_than_one_deflate_block() {
        let img = ImageBuf::solid(200, 100, Color::rgb(0.5, 0.5, 0.5)); // 80_100 raw bytes
        let chunks = parse_png(&encode_png(&img));
        let raw = inflate_stored(&chunks[1].1);
        assert_eq!(raw.len(), 100 * (1 + 800));
    }

    #[test]
    fn known_checksums() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn export_writes_files_and_refuses_unavailable_formats() {
        let img = ImageBuf::solid(4, 4, Color::rgb(1.0, 0.0, 0.0));
        let dir = std::env::temp_dir().join(format!("compnode-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        export(&img, png.to_str().unwrap(), ExportFormat::Png).unwrap();
        assert!(
            std::fs::read(&png)
                .unwrap()
                .starts_with(&[0x89, b'P', b'N', b'G'])
        );
        let ppm = dir.join("a.ppm");
        export(&img, ppm.to_str().unwrap(), ExportFormat::Ppm).unwrap();
        let back = PpmLoader.load(ppm.to_str().unwrap()).unwrap();
        assert!(img.max_abs_diff(&back).unwrap() < 1e-5);
        #[cfg(not(feature = "image-io"))]
        assert!(
            export(&img, "x.jpg", ExportFormat::Jpeg)
                .unwrap_err()
                .contains("image")
        );
        assert!(
            PpmLoader
                .load(dir.join("missing.ppm").to_str().unwrap())
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downscale_keeps_aspect_and_never_enlarges() {
        let img = ImageBuf::solid(400, 200, Color::WHITE);
        let s = downscale_to_fit(img.clone(), 100, 100);
        assert_eq!((s.width, s.height), (100, 50));
        let same = downscale_to_fit(img.clone(), 1000, 1000);
        assert_eq!((same.width, same.height), (400, 200));
        let t = thumbnail(&img, 64);
        assert_eq!((t.width, t.height), (64, 32));
        assert!((t.get(5, 5)[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn async_loader_does_not_block_and_reports_errors() {
        use std::sync::mpsc;
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate = Mutex::new(gate_rx);
        let loader = FnLoader(move |path: &str| {
            if path == "bad" {
                return Err("nope".to_string());
            }
            gate.lock().unwrap().recv().map_err(|e| e.to_string())?;
            Ok(ImageBuf::solid(800, 400, Color::WHITE))
        });
        let al = AsyncLoader::new(Arc::new(loader), 2, Some((100, 100)));
        al.submit(1, "good");
        al.submit(2, "bad");
        let first = al
            .wait(Duration::from_secs(5))
            .expect("error result arrives while the good load is gated");
        assert_eq!((first.id, first.result.is_err()), (2, true));
        assert!(al.poll().is_empty()); // the gated load has not finished
        gate_tx.send(()).unwrap();
        let second = al.wait(Duration::from_secs(5)).expect("good result");
        let img = second.result.unwrap();
        assert_eq!((second.id, img.width, img.height), (1, 100, 50));
        drop(al); // joins the workers
    }

    #[test]
    fn store_loader_serves_only_what_was_put() {
        let store = StoreLoader::new();
        let shared = store.clone();
        assert!(store.load("a").unwrap_err().contains("loading"));
        shared.put("a", ImageBuf::solid(2, 2, Color::WHITE));
        assert!(store.contains("a"));
        assert_eq!(store.load("a").unwrap().width, 2);
        store.remove("a");
        assert!(shared.load("a").is_err());
        shared.fail("a", "cannot decode");
        assert_eq!(store.load("a").unwrap_err(), "cannot decode");
        shared.put("a", ImageBuf::new(1, 1)); // a successful load clears the failure
        assert!(store.load("a").is_ok());
        store.remove("a");
        shared.put("b", ImageBuf::new(1, 1));
        store.clear();
        assert!(!shared.contains("b"));
    }
}
