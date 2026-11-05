//! The stock image nodes: schema (sockets, defaults, widgets) plus the CPU
//! implementation of each.
use crate::buffer::ImageBuf;
use crate::cpu::{self, BlendMode};
use crate::eval::{Data, EvalCtx};
use compnode_core::Color;
use compnode_ui::schema::{Category, NodeSchema, Registry, SocketDef, WidgetKind};
use compnode_ui::value::{SocketType, Value};
use std::sync::Arc;

/// A node implementation: its schema and how to compute its outputs.
pub trait NodeOp: Send + Sync {
    fn schema(&self) -> NodeSchema;
    fn eval(&self, ctx: &mut EvalCtx) -> Result<Vec<Data>, String>;
}

type EvalFn = fn(&mut EvalCtx) -> Result<Vec<Data>, String>;

struct FnOp {
    schema: NodeSchema,
    f: EvalFn,
}

impl NodeOp for FnOp {
    fn schema(&self) -> NodeSchema {
        self.schema.clone()
    }
    fn eval(&self, ctx: &mut EvalCtx) -> Result<Vec<Data>, String> {
        (self.f)(ctx)
    }
}

fn img(i: ImageBuf) -> Result<Vec<Data>, String> {
    Ok(vec![Data::Image(Arc::new(i))])
}

fn slider(name: &str, default: f32, min: f32, max: f32) -> SocketDef {
    SocketDef::float(name, default)
        .range(min, max)
        .widget(WidgetKind::Slider)
}

fn image_in() -> SocketDef {
    SocketDef::image("Image")
}

fn image_out() -> SocketDef {
    SocketDef::image("Image")
}

/// Every built-in operation.
pub fn builtin_ops() -> Vec<Box<dyn NodeOp>> {
    let mut v: Vec<Box<dyn NodeOp>> = Vec::new();
    let mut add = |schema: NodeSchema, f: EvalFn| v.push(Box::new(FnOp { schema, f }));

    add(
        NodeSchema::new("image.file", "Image File", Category::Input)
            .input(SocketDef::new("Path", SocketType::String).widget(WidgetKind::TextArea))
            .input(SocketDef::new("Browse", SocketType::Bool).widget(WidgetKind::Button))
            .output(image_out())
            .footer(SocketDef::image("Preview").widget(WidgetKind::ImagePreview)),
        |ctx| {
            let path = ctx.string(0);
            if path.trim().is_empty() {
                return Err("no file chosen".to_string());
            }
            Ok(vec![Data::Image(ctx.load_file(path.trim())?)])
        },
    );
    add(
        NodeSchema::new("image.solid", "Solid Color", Category::Input)
            .input(SocketDef::color("Color", Color::rgb(0.5, 0.5, 0.5)))
            .output(image_out()),
        |ctx| img(ImageBuf::solid(ctx.canvas.0, ctx.canvas.1, ctx.color(0))),
    );
    add(
        NodeSchema::new("image.invert", "Invert", Category::Color)
            .input(image_in())
            .input(slider("Amount", 1.0, 0.0, 1.0))
            .output(image_out()),
        |ctx| {
            img(cpu::invert(
                &ctx.image_or_transparent(0),
                ctx.float(1).clamp(0.0, 1.0),
            ))
        },
    );
    add(
        NodeSchema::new(
            "image.brightness_contrast",
            "Brightness / Contrast",
            Category::Color,
        )
        .input(image_in())
        .input(slider("Brightness", 0.0, -1.0, 1.0))
        .input(slider("Contrast", 0.0, -1.0, 1.0))
        .output(image_out()),
        |ctx| {
            img(cpu::brightness_contrast(
                &ctx.image_or_transparent(0),
                ctx.float(1),
                ctx.float(2).clamp(-1.0, 1.0),
            ))
        },
    );
    add(
        NodeSchema::new("image.levels", "Levels", Category::Color)
            .input(image_in())
            .input(SocketDef::float("In Black", 0.0))
            .input(SocketDef::float("In White", 1.0))
            .input(SocketDef::float("Gamma", 1.0).range(0.01, 10.0))
            .input(SocketDef::float("Out Black", 0.0))
            .input(SocketDef::float("Out White", 1.0))
            .output(image_out()),
        |ctx| {
            img(cpu::levels(
                &ctx.image_or_transparent(0),
                ctx.float(1),
                ctx.float(2),
                ctx.float(3),
                ctx.float(4),
                ctx.float(5),
            ))
        },
    );
    add(
        NodeSchema::new("image.curves", "Curves", Category::Color)
            .input(image_in())
            .input(slider("Shadows", 0.0, 0.0, 1.0))
            .input(slider("Darks", 0.25, 0.0, 1.0))
            .input(slider("Midtones", 0.5, 0.0, 1.0))
            .input(slider("Lights", 0.75, 0.0, 1.0))
            .input(slider("Highlights", 1.0, 0.0, 1.0))
            .output(image_out()),
        |ctx| {
            // a five-point curve: the output level at input levels 0, 1/4, 1/2, 3/4 and 1
            let pts: Vec<(f32, f32)> = (0..5).map(|i| (i as f32 / 4.0, ctx.float(i + 1))).collect();
            img(cpu::curves(&ctx.image_or_transparent(0), &pts))
        },
    );
    add(
        NodeSchema::new("image.hue_sat", "Hue / Saturation", Category::Color)
            .input(image_in())
            .input(slider("Hue", 0.0, -180.0, 180.0))
            .input(slider("Saturation", 1.0, 0.0, 2.0))
            .input(slider("Value", 1.0, 0.0, 2.0))
            .output(image_out()),
        |ctx| {
            img(cpu::hue_saturation(
                &ctx.image_or_transparent(0),
                ctx.float(1),
                ctx.float(2),
                ctx.float(3),
            ))
        },
    );
    add(
        NodeSchema::new("image.threshold", "Threshold", Category::Filter)
            .input(image_in())
            .input(slider("Threshold", 0.5, 0.0, 1.0))
            .output(image_out()),
        |ctx| img(cpu::threshold(&ctx.image_or_transparent(0), ctx.float(1))),
    );
    add(
        NodeSchema::new("image.blur", "Blur", Category::Filter)
            .input(image_in())
            .input(
                SocketDef::float("Radius", 4.0)
                    .range(0.0, 500.0)
                    .soft(0.0, 50.0),
            )
            .output(image_out()),
        |ctx| img(cpu::blur(&ctx.image_or_transparent(0), ctx.float(1))),
    );
    add(
        NodeSchema::new("image.sharpen", "Sharpen", Category::Filter)
            .input(image_in())
            .input(SocketDef::float("Amount", 1.0).range(0.0, 10.0))
            .input(SocketDef::float("Radius", 1.0).range(0.0, 50.0))
            .output(image_out()),
        |ctx| {
            img(cpu::sharpen(
                &ctx.image_or_transparent(0),
                ctx.float(1),
                ctx.float(2),
            ))
        },
    );
    let modes: Vec<&str> = BlendMode::ALL.iter().map(|m| m.name()).collect();
    add(
        NodeSchema::new("image.blend", "Blend", Category::Blend)
            .input(SocketDef::image("Base"))
            .input(SocketDef::image("Layer"))
            .input(SocketDef::image("Mask"))
            .input(
                SocketDef::new("Mode", SocketType::Enum)
                    .with_default(Value::Enum(0))
                    .options(&modes),
            )
            .input(slider("Opacity", 1.0, 0.0, 1.0))
            .output(image_out()),
        |ctx| {
            let base = ctx.image_or_transparent(0);
            let layer = ctx.image_or_transparent(1);
            let mask = ctx.image(2);
            img(cpu::blend(
                &base,
                &layer,
                BlendMode::from_index(ctx.enum_index(3)),
                ctx.float(4),
                mask.as_deref(),
            ))
        },
    );
    add(
        NodeSchema::new("image.resize", "Resize", Category::Transform)
            .input(image_in())
            .input(SocketDef::int("Width", 256).range(1.0, 8192.0))
            .input(SocketDef::int("Height", 256).range(1.0, 8192.0))
            .output(image_out()),
        |ctx| {
            img(cpu::resize(
                &ctx.image_or_transparent(0),
                ctx.int(1).clamp(1, 8192) as u32,
                ctx.int(2).clamp(1, 8192) as u32,
            ))
        },
    );
    add(
        NodeSchema::new("image.crop", "Crop", Category::Transform)
            .input(image_in())
            .input(SocketDef::int("X", 0).range(-8192.0, 8192.0))
            .input(SocketDef::int("Y", 0).range(-8192.0, 8192.0))
            .input(SocketDef::int("Width", 256).range(1.0, 8192.0))
            .input(SocketDef::int("Height", 256).range(1.0, 8192.0))
            .output(image_out()),
        |ctx| {
            img(cpu::crop(
                &ctx.image_or_transparent(0),
                i64::from(ctx.int(1)),
                i64::from(ctx.int(2)),
                ctx.int(3).clamp(1, 8192) as u32,
                ctx.int(4).clamp(1, 8192) as u32,
            ))
        },
    );
    add(
        NodeSchema::new("image.rotate", "Rotate", Category::Transform)
            .input(image_in())
            .input(SocketDef::float("Angle", 0.0).range(-360.0, 360.0))
            .output(image_out()),
        |ctx| img(cpu::rotate(&ctx.image_or_transparent(0), ctx.float(1))),
    );
    add(
        NodeSchema::new("image.output", "Output Node", Category::Output)
            .input(image_in().multi())
            .output(image_out())
            .footer(SocketDef::image("Preview").widget(WidgetKind::ImagePreview)),
        |ctx| {
            let images = ctx.images(0);
            eprintln!(
                "DBG output images={} layers={}",
                images.len(),
                ctx.layers().len()
            );
            if images.is_empty() {
                return Ok(vec![Data::Image(ctx.image_or_transparent(0))]);
            }
            // One layer per connected image, bottom-to-top in wire order.
            // Transforms are looked up per layer index (0..n).
            let paints: Vec<cpu::LayerPaint> = images
                .iter()
                .enumerate()
                .map(|(i, img)| ctx.layer_paint(i, img))
                .collect();
            let (cw, ch) = ctx.canvas;
            Ok(vec![Data::Image(Arc::new(cpu::composite_layers(
                &paints, cw, ch,
            )))])
        },
    );
    add(
        NodeSchema::new("util.float", "Float", Category::Utility)
            .input(SocketDef::float("Value", 0.0))
            .output(SocketDef::new("Value", SocketType::Float)),
        |ctx| Ok(vec![Data::Val(Value::Float(ctx.float(0)))]),
    );
    add(
        NodeSchema::new("util.color", "Color", Category::Utility)
            .input(SocketDef::color("Color", Color::rgb(0.5, 0.5, 0.5)))
            .output(SocketDef::new("Color", SocketType::Color)),
        |ctx| Ok(vec![Data::Val(Value::Color(ctx.color(0)))]),
    );
    v
}

/// Add every built-in schema to `reg`.
pub fn register_all(reg: &mut Registry) {
    for op in builtin_ops() {
        reg.register(op.schema());
    }
}

/// A registry with the built-in layout nodes and every image node.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    register_all(&mut r);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_titles_searchable() {
        let ops = builtin_ops();
        let mut ids: Vec<String> = ops.iter().map(|o| o.schema().id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        let r = registry();
        let hits: Vec<String> = r.search("blu").iter().map(|s| s.id.clone()).collect();
        assert_eq!(hits, vec!["image.blur".to_string()]);
        assert!(r.search("hue").iter().any(|s| s.id == "image.hue_sat"));
        for s in r.iter() {
            s.validate().unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn blend_dropdown_lists_every_mode() {
        let r = registry();
        let s = r.get("image.blend").unwrap();
        let mode = s.inputs.iter().find(|d| d.name == "Mode").unwrap();
        assert_eq!(mode.options.len(), BlendMode::ALL.len());
        assert_eq!(mode.widget, WidgetKind::Dropdown);
    }
}
