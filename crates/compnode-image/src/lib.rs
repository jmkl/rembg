//! # compnode-image
//!
//! Image nodes for [`compnode_ui`]: the stock node set, a dirty-propagating
//! evaluator with a memory-budgeted result cache, CPU reference
//! implementations of every operation (the oracles for a future GPU
//! backend), file loading/export and a backend-independent texture pool.
//!
//! The GPU (wgpu) backend is **not** part of this crate yet.

#![forbid(unsafe_code)]
// The per-channel loops index several arrays with the same index on purpose.
#![allow(clippy::needless_range_loop)]

pub mod buffer;
pub mod cpu;
pub mod eval;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "image-io")]
pub mod imageio;
pub mod loader;
pub mod nodes;
pub mod pool;
pub mod thumbs;

pub use buffer::ImageBuf;
pub use cpu::BlendMode;
pub use eval::{Data, EvalCtx, EvalError, Evaluator};
pub use loader::{AsyncLoader, ExportFormat, ImageLoader, PpmLoader};
pub use nodes::{NodeOp, registry};
pub use pool::{TexKey, TexturePool};
