//! Project files: a small line-based text format for the graph and its
//! parameters. Images are referenced by path (the Image File node keeps its
//! path as an ordinary string value); no pixels are stored.
//!
//! ```text
//! compnode-project 1
//! node 3 image.blur 400 0 180 200 0 0 -
//! value 3 1 f 4
//! link 2 0 3 0
//! ```
use crate::document::{Document, Layer, Link, NodeSnapshot};
use crate::schema::Registry;
use crate::value::{ImageHandle, Value};
use compnode_core::Color;
use compnode_core::geom::vec2;

const HEADER: &str = "compnode-project 1";

fn esc(s: &str) -> String {
    if s.is_empty() {
        return "\\e".to_string();
    }
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            ' ' => o.push_str("\\s"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> Result<String, String> {
    if s == "\\e" {
        return Ok(String::new());
    }
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('\\') => o.push('\\'),
            Some('s') => o.push(' '),
            Some('n') => o.push('\n'),
            Some('t') => o.push('\t'),
            Some('r') => o.push('\r'),
            other => {
                return Err(format!(
                    "bad escape \\{}",
                    other.map_or(String::new(), |c| c.to_string())
                ));
            }
        }
    }
    Ok(o)
}

fn value_to_text(v: &Value) -> String {
    match v {
        Value::Float(f) => format!("f {f:?}"),
        Value::Int(i) => format!("i {i}"),
        Value::Bool(b) => format!("b {}", u8::from(*b)),
        Value::Vec3(a) => format!("v3 {:?} {:?} {:?}", a[0], a[1], a[2]),
        Value::Color(c) => format!("c {:?} {:?} {:?} {:?}", c.r, c.g, c.b, c.a),
        Value::String(s) => format!("s {}", esc(s)),
        Value::Enum(i) => format!("e {i}"),
        Value::Image(_) => "img".to_string(),
    }
}

fn value_from_tokens(t: &[&str]) -> Result<Value, String> {
    let num = |i: usize| -> Result<f32, String> {
        t.get(i)
            .ok_or("missing number")?
            .parse::<f32>()
            .map_err(|e| format!("bad number: {e}"))
    };
    let kind = *t.first().ok_or("missing value kind")?;
    Ok(match kind {
        "f" => Value::Float(num(1)?),
        "i" => Value::Int(
            t.get(1)
                .ok_or("missing int")?
                .parse::<i32>()
                .map_err(|e| format!("bad int: {e}"))?,
        ),
        "b" => Value::Bool(*t.get(1).ok_or("missing bool")? == "1"),
        "v3" => Value::Vec3([num(1)?, num(2)?, num(3)?]),
        "c" => Value::Color(Color::new(num(1)?, num(2)?, num(3)?, num(4)?)),
        "s" => Value::String(unesc(t.get(1).ok_or("missing string")?)?),
        "e" => Value::Enum(
            t.get(1)
                .ok_or("missing enum")?
                .parse::<usize>()
                .map_err(|e| format!("bad enum: {e}"))?,
        ),
        "img" => Value::Image(None::<ImageHandle>),
        other => return Err(format!("unknown value kind {other:?}")),
    })
}

/// Serialize the document (nodes sorted by uid, so output is deterministic).
pub fn save(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push('\n');
    for uid in doc.uids() {
        let Some(s) = doc.snapshot_node(uid) else {
            continue;
        };
        let title = s.title.as_deref().map_or_else(|| "-".to_string(), esc);
        out.push_str(&format!(
            "node {} {} {:?} {:?} {:?} {:?} {} {} {}\n",
            s.uid,
            s.schema,
            s.pos.x,
            s.pos.y,
            s.width,
            s.frame_h,
            u8::from(s.muted),
            u8::from(s.collapsed),
            title
        ));
        for (i, v) in s.values.iter().enumerate() {
            out.push_str(&format!("value {} {} {}\n", s.uid, i, value_to_text(v)));
        }
        // Layer transforms (multi-input canvas nodes), bottom-to-top.
        for (i, l) in s.layers.iter().enumerate() {
            out.push_str(&format!(
                "layer {} {} {} {:?} {:?} {:?} {:?}\n",
                s.uid, i, l.source, l.center[0], l.center[1], l.size[0], l.size[1]
            ));
            if l.angle != 0.0 {
                out.push_str(&format!("layer_angle {} {} {:?}\n", s.uid, i, l.angle));
            }
        }
    }
    for l in doc.all_links() {
        out.push_str(&format!(
            "link {} {} {} {}\n",
            l.from.0, l.from.1, l.to.0, l.to.1
        ));
    }
    out
}

/// Parse a project. Unknown node kinds, bad lines, duplicate uids and cycles
/// are errors that name the line.
pub fn load(text: &str, registry: Registry) -> Result<Document, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim() == HEADER => {}
        _ => return Err(format!("not a compnode project (expected {HEADER:?})")),
    }
    let mut doc = Document::new(registry);
    let mut links: Vec<Link> = Vec::new();
    let mut nodes: Vec<NodeSnapshot> = Vec::new();
    for (n, raw) in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let at = |e: String| format!("line {}: {e}", n + 1);
        let t: Vec<&str> = line.split_whitespace().collect();
        let int = |i: usize| -> Result<u64, String> {
            t.get(i)
                .ok_or("missing field")?
                .parse::<u64>()
                .map_err(|e| format!("bad integer: {e}"))
        };
        let flt = |i: usize| -> Result<f32, String> {
            t.get(i)
                .ok_or("missing field")?
                .parse::<f32>()
                .map_err(|e| format!("bad number: {e}"))
        };
        match t[0] {
            "node" => {
                let uid = int(1).map_err(at)?;
                let schema_id = t.get(2).ok_or_else(|| at("missing schema".into()))?;
                let schema = doc
                    .registry
                    .get(schema_id)
                    .ok_or_else(|| at(format!("unknown node kind {schema_id:?}")))?;
                if nodes.iter().any(|s| s.uid == uid) {
                    return Err(at(format!("duplicate node {uid}")));
                }
                let title = match t.get(9).ok_or_else(|| at("missing title".into()))? {
                    &"-" => None,
                    other => Some(unesc(other).map_err(at)?),
                };
                nodes.push(NodeSnapshot {
                    uid,
                    schema: schema.id.clone(),
                    pos: vec2(flt(3).map_err(at)?, flt(4).map_err(at)?),
                    width: flt(5).map_err(at)?,
                    frame_h: flt(6).map_err(at)?,
                    values: schema.default_values(),
                    out_values: schema.default_out_values(),
                    muted: int(7).map_err(at)? == 1,
                    collapsed: int(8).map_err(at)? == 1,
                    title,
                    layers: Vec::new(),
                });
            }
            "value" => {
                let uid = int(1).map_err(at)?;
                let idx = int(2).map_err(at)? as usize;
                let v = value_from_tokens(&t[3..]).map_err(at)?;
                let node = nodes
                    .iter_mut()
                    .find(|s| s.uid == uid)
                    .ok_or_else(|| at(format!("value for unknown node {uid}")))?;
                let slot = node
                    .values
                    .get_mut(idx)
                    .ok_or_else(|| at(format!("node {uid} has no input {idx}")))?;
                // the image value is a runtime handle: keep the default
                if !matches!(v, Value::Image(_)) {
                    if v.socket_type() != slot.socket_type() {
                        return Err(at(format!(
                            "value type mismatch for node {uid} input {idx}"
                        )));
                    }
                    *slot = v;
                }
            }
            "link" => {
                links.push(Link {
                    from: (int(1).map_err(at)?, int(2).map_err(at)? as usize),
                    to: (int(3).map_err(at)?, int(4).map_err(at)? as usize),
                });
            }
            "layer" => {
                let uid = int(1).map_err(at)?;
                let idx = int(2).map_err(at)? as usize;
                let source = int(3).map_err(at)?;
                let layer = Layer {
                    source,
                    center: [flt(4).map_err(at)?, flt(5).map_err(at)?],
                    size: [flt(6).map_err(at)?, flt(7).map_err(at)?],
                    angle: 0.0,
                };
                let node = nodes
                    .iter_mut()
                    .find(|s| s.uid == uid)
                    .ok_or_else(|| at(format!("layer for unknown node {uid}")))?;
                if idx != node.layers.len() {
                    return Err(at(format!("layer {idx} out of order for node {uid}")));
                }
                node.layers.push(layer);
            }
            "layer_angle" => {
                let uid = int(1).map_err(at)?;
                let idx = int(2).map_err(at)? as usize;
                let angle = flt(3).map_err(at)?;
                let node = nodes
                    .iter_mut()
                    .find(|s| s.uid == uid)
                    .ok_or_else(|| at(format!("layer_angle for unknown node {uid}")))?;
                let layer = node.layers.get_mut(idx).ok_or_else(|| {
                    at(format!("layer_angle for missing layer {idx} of node {uid}"))
                })?;
                layer.angle = angle;
            }
            other => return Err(at(format!("unknown record {other:?}"))),
        }
    }
    for s in &nodes {
        doc.insert_snapshot(s)
            .ok_or_else(|| format!("could not create node {}", s.uid))?;
    }
    doc.load_links(&links)
        .map_err(|e| format!("links rejected: {e:?}"))?;
    doc.sync_all_layers();
    doc.take_changes();
    doc.take_layout_dirty();
    // everything needs a layout pass in the editor after loading
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::test_support::registry;

    fn sample() -> Document {
        let mut d = Document::new(registry());
        let a = d.add_node("t.src", vec2(10.5, -20.25)).unwrap();
        let f = d.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        let s = d.add_node("t.sink", vec2(800.0, 0.0)).unwrap();
        d.connect(Link {
            from: (a, 0),
            to: (f, 0),
        })
        .unwrap();
        d.connect(Link {
            from: (f, 0),
            to: (s, 0),
        })
        .unwrap();
        d.set_value(f, 1, Value::Float(0.1 + 0.2));
        d.set_muted(f, true);
        d
    }

    #[test]
    fn roundtrip_is_exact_and_stable() {
        let d = sample();
        let text = save(&d);
        let back = load(&text, registry()).expect("loads");
        assert_eq!(save(&back), text);
        assert_eq!(back.uids(), d.uids());
        assert_eq!(back.all_links(), d.all_links());
        let f = d.uids()[1];
        assert_eq!(back.data(f).unwrap().values, d.data(f).unwrap().values);
        assert!(back.data(f).unwrap().muted);
        assert_eq!(back.pos(d.uids()[0]), Some(vec2(10.5, -20.25)));
        // new uids continue after the loaded ones
        let mut back = back;
        assert!(back.add_node("t.src", vec2(0.0, 0.0)).unwrap() > *d.uids().last().unwrap());
    }

    #[test]
    fn all_value_kinds_roundtrip() {
        for v in [
            Value::Float(-0.0),
            Value::Float(1.0e-7),
            Value::Int(-5),
            Value::Bool(true),
            Value::Vec3([1.0, 2.5, -3.0]),
            Value::Color(Color::new(0.1, 0.2, 0.3, 0.4)),
            Value::String("a b\nc\\d\t".into()),
            Value::String(String::new()),
            Value::Enum(3),
        ] {
            let text = value_to_text(&v);
            let toks: Vec<&str> = text.split_whitespace().collect();
            assert_eq!(value_from_tokens(&toks).unwrap(), v, "{text}");
        }
    }

    #[test]
    fn errors_name_the_line() {
        assert!(load("nope", registry()).is_err());
        let e = load(
            "compnode-project 1\nnode 1 does.not.exist 0 0 1 1 0 0 -\n",
            registry(),
        )
        .unwrap_err();
        assert!(
            e.contains("line 2") && e.contains("unknown node kind"),
            "{e}"
        );
        let e = load("compnode-project 1\nvalue 9 0 f 1\n", registry()).unwrap_err();
        assert!(e.contains("unknown node"), "{e}");
        let e = load(
            "compnode-project 1\nnode 1 t.src 0 0 1 1 0 0 -\nvalue 1 0 s hi\n",
            registry(),
        )
        .unwrap_err();
        assert!(e.contains("mismatch"), "{e}");
        let e = load(
            "compnode-project 1\nnode 1 t.src 0 0 1 1 0 0 -\nnode 1 t.src 0 0 1 1 0 0 -\n",
            registry(),
        )
        .unwrap_err();
        assert!(e.contains("duplicate"), "{e}");
        let cyc = "compnode-project 1\nnode 1 t.filter 0 0 1 1 0 0 -\nnode 2 t.filter 0 0 1 1 0 0 -\nlink 1 0 2 0\nlink 2 0 1 0\n";
        assert!(load(cyc, registry()).is_err());
    }

    #[test]
    fn titles_survive() {
        let mut d = sample();
        let a = d.uids()[0];
        let mut s = d.snapshot_node(a).unwrap();
        d.remove_node(a);
        s.title = Some("My source node".into());
        d.insert_snapshot(&s).unwrap();
        let back = load(&save(&d), registry()).unwrap();
        assert_eq!(
            back.data(a).unwrap().title.as_deref(),
            Some("My source node")
        );
    }
}
