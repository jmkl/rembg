//! wgpu backend. Written against the wgpu 29 API; see the notes marked
//! `API:` for the spots that changed between wgpu versions.
//!
//! The whole pipeline is recorded into ONE command encoder:
//!   upload -> unpack -> [NR] -> [WB+dehaze] -> [texture] -> [clarity]
//!          -> [sharpen] -> pack -> readback
//! Data never leaves the GPU between stages and there are no u8 round-trips.
//!
//! Memory: 3 x vec4<f32> work buffers + packed io = ~52 bytes/pixel. Images
//! whose work buffers exceed the device's storage-binding limit return `None`
//! so the caller falls back to the CPU path.

use std::sync::OnceLock;

use image::RgbaImage;
use wgpu::util::DeviceExt;

use super::Settings;

const WG: u32 = 8; // must match @workgroup_size in the shader

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// Returns `None` if there is no usable adapter, the image does not fit, or
/// readback fails. Device creation happens once, on first use.
pub(super) fn apply(src: &RgbaImage, s: &Settings) -> Option<RgbaImage> {
    GPU.get_or_init(Gpu::new).as_ref()?.run(src, s)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    size: [u32; 4],
    f0: [f32; 4],
    f1: [f32; 4],
}

struct Pipelines {
    unpack: wgpu::ComputePipeline,
    pack: wgpu::ComputePipeline,
    blur_h: wgpu::ComputePipeline,
    blur_v: wgpu::ComputePipeline,
    nr_apply: wgpu::ComputePipeline,
    color: wgpu::ComputePipeline,
    local_contrast: wgpu::ComputePipeline,
    sharpen_apply: wgpu::ComputePipeline,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Pipelines,
    max_bytes: u64,
}

struct Bufs {
    io: wgpu::Buffer,
    img: wgpu::Buffer,
    t0: wgpu::Buffer,
    t1: wgpu::Buffer,
}

impl Gpu {
    fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();

        // API: wgpu <= 24 returns Option here instead of Result.
        let adapter = ok(
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })),
            "request_adapter",
        )?;
        eprintln!("[raw_filter gpu] adapter: {:?}", adapter.get_info());
        // Ask for everything the adapter offers: the default 128 MiB storage
        // binding limit would cap us at ~8 MP.
        let limits = adapter.limits();
        let max_bytes = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);

        // API: wgpu <= 24 takes a second `trace_path` argument.
        let (device, queue) = ok(
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("raw_filter"),
                required_limits: limits,
                ..Default::default()
            })),
            "request_device",
        )?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raw_filter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("raw_filter.wgsl").into()),
        });

        // Auto layouts: each entry point only exposes the bindings it uses.
        let make = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };

        let pipelines = Pipelines {
            unpack: make("unpack"),
            pack: make("pack"),
            blur_h: make("blur_h"),
            blur_v: make("blur_v"),
            nr_apply: make("nr_apply"),
            color: make("color"),
            local_contrast: make("local_contrast"),
            sharpen_apply: make("sharpen_apply"),
        };

        Some(Self {
            device,
            queue,
            pipelines,
            max_bytes,
        })
    }

    fn run(&self, src: &RgbaImage, s: &Settings) -> Option<RgbaImage> {
        let (w, h) = src.dimensions();
        let pixels = w as u64 * h as u64;
        let io_bytes = pixels * 4;
        let work_bytes = pixels * 16;
        if work_bytes > self.max_bytes || w.div_ceil(WG) > 65_535 || h.div_ceil(WG) > 65_535 {
            eprintln!(
                "[raw_filter gpu] image too large: need {work_bytes} B, limit {} B",
                self.max_bytes
            );
            return None;
        }

        let storage = wgpu::BufferUsages::STORAGE;
        let work = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: work_bytes,
                usage: storage,
                mapped_at_creation: false,
            })
        };
        let bufs = Bufs {
            // RGBA8 bytes read as little-endian u32: R in the low byte, which
            // is what unpack4x8unorm / pack4x8unorm expect.
            io: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("io"),
                    contents: src.as_raw(),
                    usage: storage | wgpu::BufferUsages::COPY_SRC,
                }),
            img: work("img"),
            t0: work("t0"),
            t1: work("t1"),
        };
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: io_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut job = Job {
            gpu: self,
            enc: self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("raw_filter"),
                }),
            w,
            h,
        };
        let p = &self.pipelines;
        let z = [0.0; 4];
        let img_only = [(2, &bufs.img)];
        let img_t1 = [(2, &bufs.img), (4, &bufs.t1)];

        job.pass(&p.unpack, z, z, &[(1, &bufs.io), (2, &bufs.img)]);

        // Same stage order as the CPU path / original implementation.
        if s.nr_amount() > 0.0 {
            job.blur(1.2, &bufs);
            job.pass(
                &p.nr_apply,
                [s.nr_amount(), s.nr_threshold(), 0.0, 0.0],
                z,
                &img_t1,
            );
        }
        if s.wb_active() || s.dehaze_amount() != 0.0 {
            let g = s.wb_gains();
            let wb_on = if s.wb_active() { 1.0 } else { 0.0 };
            job.pass(
                &p.color,
                [g[0], g[1], g[2], wb_on],
                [s.dehaze_amount(), 0.0, 0.0, 0.0],
                &img_only,
            );
        }
        if s.texture_amount() != 0.0 {
            job.blur(1.2, &bufs);
            job.pass(
                &p.local_contrast,
                [s.texture_amount(), 0.0, 0.0, 0.0],
                z,
                &img_t1,
            );
        }
        if s.clarity_amount() != 0.0 {
            job.blur(6.0, &bufs);
            job.pass(
                &p.local_contrast,
                [s.clarity_amount(), 0.6, 0.0, 0.0],
                z,
                &img_t1,
            );
        }
        if s.sharpen > 0.0 {
            job.blur(s.sharpen_sigma(), &bufs);
            job.pass(
                &p.sharpen_apply,
                [s.sharpen_strength(), s.sharpen_threshold(), 0.0, 0.0],
                z,
                &img_t1,
            );
        }

        job.pass(&p.pack, z, z, &[(1, &bufs.io), (2, &bufs.img)]);

        let mut enc = job.enc;
        enc.copy_buffer_to_buffer(&bufs.io, 0, &readback, 0, io_bytes);
        self.queue.submit([enc.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });

        // API: wgpu 22-24 `device.poll(wgpu::Maintain::Wait)`,
        //      wgpu 25 `device.poll(wgpu::PollType::Wait)`.
        ok(
            self.device.poll(wgpu::PollType::wait_indefinitely()),
            "poll",
        )?;
        ok(rx.recv(), "map recv")?
            .map_err(|e| eprintln!("[raw_filter gpu] map failed: {e:?}"))
            .ok()?;

        let bytes = slice.get_mapped_range().to_vec();
        readback.unmap();
        RgbaImage::from_raw(w, h, bytes)
    }
}

/// Records compute passes into one encoder.
struct Job<'a> {
    gpu: &'a Gpu,
    enc: wgpu::CommandEncoder,
    w: u32,
    h: u32,
}

impl<'a> Job<'a> {
    /// One compute pass. `bufs` are (binding, buffer) pairs; binding 0 is the
    /// per-pass uniform. Each pass gets its own uniform buffer because
    /// `queue.write_buffer` calls all land before the submitted work runs.
    fn pass(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        f0: [f32; 4],
        f1: [f32; 4],
        bufs: &[(u32, &wgpu::Buffer)],
    ) {
        let gpu = self.gpu;
        let uniforms = Uniforms {
            size: [self.w, self.h, 0, 0],
            f0,
            f1,
        };
        let ubuf = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: ubuf.as_entire_binding(),
        }];
        entries.extend(bufs.iter().map(|(binding, buf)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: buf.as_entire_binding(),
        }));
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });

        let mut cpass = self
            .enc
            .begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        cpass.dispatch_workgroups(self.w.div_ceil(WG), self.h.div_ceil(WG), 1);
    }

    /// Gaussian blur of `img` into `t1` (via `t0`).
    fn blur(&mut self, sigma: f32, b: &Bufs) {
        let gpu = self.gpu;
        let f0 = [sigma, 0.0, 0.0, 0.0];
        self.pass(
            &gpu.pipelines.blur_h,
            f0,
            [0.0; 4],
            &[(2, &b.img), (3, &b.t0)],
        );
        self.pass(
            &gpu.pipelines.blur_v,
            f0,
            [0.0; 4],
            &[(3, &b.t0), (4, &b.t1)],
        );
    }
}

fn ok<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> Option<T> {
    r.map_err(|e| eprintln!("[raw_filter gpu] {what} failed: {e:?}"))
        .ok()
}
