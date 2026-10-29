# Pixel Data Contract

This document records the pixel behavior currently used by CompNode operations. It is the baseline for future color or alpha changes; it does not imply that all operations are physically linear-light or color-managed.

## Image representation

- The common interchange type is `image::RgbaImage`: four 8-bit channels in RGBA order.
- RGB bytes are treated as encoded channel values (normally sRGB-like image data). The current processing path does not read or apply ICC profiles and does not linearize RGB before arithmetic.
- `RgbaImage` pixel channels use straight, or unassociated, alpha unless an operation explicitly documents an internal representation.
- Most color adjustments change RGB and preserve the input alpha byte. Generators produce opaque pixels except the Texts generator, whose background remains transparent.

## Alpha and compositing

- The Texts renderer uses a premultiplied floating-point canvas while painting, then emits straight-alpha RGBA bytes for its output.
- Layer compositing uses straight-alpha source-over equations and the selected blend function on encoded RGB channel values.
- Bilinear and bicubic samplers interpolate each RGBA channel independently. They do not premultiply RGB by alpha before filtering, so transparent-edge color bleed is possible and is part of current behavior.
- Alpha masks derive luminance from the mask RGB channels and ignore the mask's own alpha channel. The source alpha is multiplied by the resulting mask factor.
- Raw Filter preserves input alpha on both CPU and GPU paths.

## Luma and conversion rules

- Raw Filter, color balance, gradient-map input, and mask luminance use Rec. 709 coefficients (`0.2126, 0.7152, 0.0722`).
- CSS blend luminosity uses the CSS blend coefficients (`0.3, 0.59, 0.11`) to retain existing blend-mode behavior. The Rec. 601 helper is separately named and must not silently replace either definition.
- Shared normalized-channel conversion clamps to `[0, 1]` and rounds to the nearest byte. Shared byte-range conversion clamps to `[0, 255]` and rounds to the nearest integer.
- Byte-to-float conversion divides by 255. Operations that need a different historical rule must use a clearly named helper and have a regression case.

## Bounds, malformed values, and determinism

- Shared samplers clamp finite coordinates to the image edges. Empty images, non-finite sample coordinates, and undersized pixel buffers return transparent black.
- Noise output follows the requested canvas dimensions, with zero edges raised to 1; gradients clamp requested dimensions to `100..=2000` per edge. Noise relies on the caller to bound unusually large canvases.
- Typed floating-point parameter reads use their supplied default when conversion produces a non-finite `f32` (for example, an extremely large JSON number). Operation-specific ranges are clamped by the operation.
- Noise output is repeatable for the same parameters and seed in the same build/runtime. This is not a promise of bit-identical output across platforms or future algorithm changes.

## Compatibility rule

Changing color-space conversion, alpha filtering, mask interpretation, or quantization can change existing images. Such changes require golden input/output cases, a documented compatibility decision, and CPU/GPU parity checks where both backends exist.
