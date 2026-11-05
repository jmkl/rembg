//! Undo/redo. Every mutation is a [`Command`] with `apply` and `invert`.
//! Commands refer to nodes by uid, so they stay valid across delete/undo.
use crate::document::{Document, Link, Snapshot};
use crate::value::Value;
use compnode_core::Vec2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flag {
    Muted,
    Collapsed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Add nodes and the links between them.
    AddNodes(Snapshot),
    /// Remove nodes; the snapshot holds everything needed to bring them back.
    RemoveNodes(Snapshot),
    /// `(uid, old, new)` per node.
    MoveNodes(Vec<(u64, Vec2, Vec2)>),
    SetValue {
        uid: u64,
        input: usize,
        old: Value,
        new: Value,
    },
    /// Remove `remove`, then add `add`. Anything already on an added link's
    /// input must be listed in `remove` so the command can be undone.
    SetLinks {
        remove: Vec<Link>,
        add: Vec<Link>,
    },
    /// `(width, frame content height)`.
    Resize {
        uid: u64,
        old: (f32, f32),
        new: (f32, f32),
    },
    SetFlag {
        uid: u64,
        flag: Flag,
        old: bool,
        new: bool,
    },
    Transaction(Vec<Command>),
}

impl Command {
    pub fn apply(&self, doc: &mut Document) {
        match self {
            Command::AddNodes(s) => {
                for n in &s.nodes {
                    doc.insert_snapshot(n);
                }
                for l in &s.links {
                    let _ = doc.connect(*l);
                }
            }
            Command::RemoveNodes(s) => {
                for n in &s.nodes {
                    doc.remove_node(n.uid);
                }
            }
            Command::MoveNodes(m) => {
                for (uid, _, new) in m {
                    doc.set_pos(*uid, *new);
                }
            }
            Command::SetValue {
                uid, input, new, ..
            } => {
                doc.set_value(*uid, *input, new.clone());
            }
            Command::SetLinks { remove, add } => {
                for l in remove {
                    doc.disconnect(*l);
                }
                for l in add {
                    let _ = doc.connect(*l);
                }
            }
            Command::Resize { uid, new, .. } => {
                doc.set_size(*uid, new.0, new.1);
            }
            Command::SetFlag { uid, flag, new, .. } => match flag {
                Flag::Muted => {
                    doc.set_muted(*uid, *new);
                }
                Flag::Collapsed => {
                    doc.set_collapsed(*uid, *new);
                }
            },
            Command::Transaction(v) => {
                for c in v {
                    c.apply(doc);
                }
            }
        }
    }

    pub fn invert(&self) -> Command {
        match self {
            Command::AddNodes(s) => Command::RemoveNodes(s.clone()),
            Command::RemoveNodes(s) => Command::AddNodes(s.clone()),
            Command::MoveNodes(m) => {
                Command::MoveNodes(m.iter().map(|(u, o, n)| (*u, *n, *o)).collect())
            }
            Command::SetValue {
                uid,
                input,
                old,
                new,
            } => Command::SetValue {
                uid: *uid,
                input: *input,
                old: new.clone(),
                new: old.clone(),
            },
            Command::SetLinks { remove, add } => Command::SetLinks {
                remove: add.clone(),
                add: remove.clone(),
            },
            Command::Resize { uid, old, new } => Command::Resize {
                uid: *uid,
                old: *new,
                new: *old,
            },
            Command::SetFlag {
                uid,
                flag,
                old,
                new,
            } => Command::SetFlag {
                uid: *uid,
                flag: *flag,
                old: *new,
                new: *old,
            },
            Command::Transaction(v) => {
                Command::Transaction(v.iter().rev().map(Command::invert).collect())
            }
        }
    }

    /// Fold `next` into `self` when both are the same kind of edit on the
    /// same target (slider scrubbing, drags). Returns `true` if merged.
    pub fn try_merge(&mut self, next: &Command) -> bool {
        match (self, next) {
            (
                Command::SetValue {
                    uid, input, new, ..
                },
                Command::SetValue {
                    uid: u2,
                    input: i2,
                    new: n2,
                    ..
                },
            ) if uid == u2 && input == i2 => {
                *new = n2.clone();
                true
            }
            (Command::MoveNodes(a), Command::MoveNodes(b))
                if a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.0 == y.0) =>
            {
                for (x, y) in a.iter_mut().zip(b.iter()) {
                    x.2 = y.2;
                }
                true
            }
            (
                Command::Resize { uid, new, .. },
                Command::Resize {
                    uid: u2, new: n2, ..
                },
            ) if uid == u2 => {
                *new = *n2;
                true
            }
            _ => false,
        }
    }

    /// Does applying this change nothing at all?
    pub fn is_noop(&self) -> bool {
        match self {
            Command::AddNodes(s) | Command::RemoveNodes(s) => s.nodes.is_empty(),
            Command::MoveNodes(m) => m.iter().all(|(_, o, n)| o == n),
            Command::SetValue { old, new, .. } => old == new,
            Command::SetLinks { remove, add } => remove.is_empty() && add.is_empty(),
            Command::Resize { old, new, .. } => old == new,
            Command::SetFlag { old, new, .. } => old == new,
            Command::Transaction(v) => v.iter().all(Command::is_noop),
        }
    }
}

/// Merge neighbours that fold together and drop no-ops.
pub fn compact(cmds: Vec<Command>) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::new();
    for c in cmds {
        if let Some(last) = out.last_mut()
            && last.try_merge(&c)
        {
            continue;
        }
        out.push(c);
    }
    out.retain(|c| !c.is_noop());
    out
}

pub struct History {
    undo: Vec<Command>,
    redo: Vec<Command>,
    pending: Vec<Command>,
    depth: usize,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        History::new()
    }
}

impl History {
    pub fn new() -> History {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            pending: Vec::new(),
            depth: 0,
            limit: 500,
        }
    }
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn in_transaction(&self) -> bool {
        self.depth > 0
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending.clear();
        self.depth = 0;
    }

    /// Start grouping. Calls nest; the group closes with the matching `end`.
    pub fn begin(&mut self) {
        self.depth += 1;
    }

    pub fn end(&mut self) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth > 0 {
            return;
        }
        let mut cmds = compact(std::mem::take(&mut self.pending));
        match cmds.len() {
            0 => {}
            1 => self.push(cmds.remove(0)),
            _ => self.push(Command::Transaction(cmds)),
        }
    }

    fn push(&mut self, c: Command) {
        self.redo.clear();
        self.undo.push(c);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Record a command that has already been applied.
    pub fn record(&mut self, c: Command) {
        if self.depth > 0 {
            self.pending.push(c);
        } else if !c.is_noop() {
            self.push(c);
        }
    }

    /// Apply and record.
    pub fn execute(&mut self, doc: &mut Document, c: Command) {
        c.apply(doc);
        self.record(c);
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if self.depth > 0 {
            return false;
        }
        let Some(c) = self.undo.pop() else {
            return false;
        };
        c.invert().apply(doc);
        self.redo.push(c);
        true
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if self.depth > 0 {
            return false;
        }
        let Some(c) = self.redo.pop() else {
            return false;
        };
        c.apply(doc);
        self.undo.push(c);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::test_support::registry;
    use compnode_core::geom::vec2;

    fn doc() -> Document {
        Document::new(registry())
    }

    fn state(d: &Document) -> (Vec<u64>, Vec<Link>) {
        (d.uids(), d.all_links())
    }

    #[test]
    fn add_remove_roundtrip_with_external_links() {
        let mut d = doc();
        let mut h = History::new();
        let a = d.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = d.add_node("t.filter", vec2(300.0, 0.0)).unwrap();
        let c = d.add_node("t.sink", vec2(600.0, 0.0)).unwrap();
        d.connect(Link {
            from: (a, 0),
            to: (b, 0),
        })
        .unwrap();
        d.connect(Link {
            from: (b, 0),
            to: (c, 0),
        })
        .unwrap();
        let before = state(&d);
        let snap = d.snapshot_nodes(&[b], true);
        h.execute(&mut d, Command::RemoveNodes(snap));
        assert_eq!(d.uids(), vec![a, c]);
        assert!(d.all_links().is_empty());
        assert!(h.undo(&mut d));
        assert_eq!(state(&d), before);
        assert!(h.redo(&mut d));
        assert_eq!(d.uids(), vec![a, c]);
        assert!(!h.redo(&mut d));
    }

    #[test]
    fn set_links_restores_replaced_link_on_undo() {
        let mut d = doc();
        let mut h = History::new();
        let a = d.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let a2 = d.add_node("t.src", vec2(0.0, 100.0)).unwrap();
        let b = d.add_node("t.filter", vec2(300.0, 0.0)).unwrap();
        let l1 = Link {
            from: (a, 0),
            to: (b, 0),
        };
        let l2 = Link {
            from: (a2, 0),
            to: (b, 0),
        };
        h.execute(
            &mut d,
            Command::SetLinks {
                remove: vec![],
                add: vec![l1],
            },
        );
        h.execute(
            &mut d,
            Command::SetLinks {
                remove: vec![l1],
                add: vec![l2],
            },
        );
        assert_eq!(d.all_links(), vec![l2]);
        h.undo(&mut d);
        assert_eq!(d.all_links(), vec![l1]);
        h.undo(&mut d);
        assert!(d.all_links().is_empty());
        h.redo(&mut d);
        h.redo(&mut d);
        assert_eq!(d.all_links(), vec![l2]);
    }

    #[test]
    fn transaction_merges_scrubbing_into_one_undo_step() {
        let mut d = doc();
        let mut h = History::new();
        let a = d.add_node("t.filter", vec2(0.0, 0.0)).unwrap();
        h.begin();
        let mut cur = Value::Float(2.0);
        for x in [3.0, 4.0, 5.0, 6.0] {
            let new = Value::Float(x);
            h.execute(
                &mut d,
                Command::SetValue {
                    uid: a,
                    input: 1,
                    old: cur.clone(),
                    new: new.clone(),
                },
            );
            cur = new;
        }
        h.end();
        assert_eq!(h.undo_len(), 1);
        assert_eq!(d.data(a).unwrap().values[1], Value::Float(6.0));
        h.undo(&mut d);
        assert_eq!(d.data(a).unwrap().values[1], Value::Float(2.0));
        h.redo(&mut d);
        assert_eq!(d.data(a).unwrap().values[1], Value::Float(6.0));
    }

    #[test]
    fn noop_transactions_leave_no_history_and_single_commands_unwrap() {
        let mut d = doc();
        let mut h = History::new();
        let a = d.add_node("t.src", vec2(10.0, 10.0)).unwrap();
        h.begin();
        h.record(Command::MoveNodes(vec![(
            a,
            vec2(10.0, 10.0),
            vec2(10.0, 10.0),
        )]));
        h.end();
        assert!(!h.can_undo());
        h.begin();
        h.execute(
            &mut d,
            Command::MoveNodes(vec![(a, vec2(10.0, 10.0), vec2(20.0, 20.0))]),
        );
        h.execute(
            &mut d,
            Command::MoveNodes(vec![(a, vec2(20.0, 20.0), vec2(30.0, 30.0))]),
        );
        h.end();
        assert_eq!(h.undo_len(), 1);
        assert!(matches!(h.undo.last(), Some(Command::MoveNodes(_))));
        h.undo(&mut d);
        assert_eq!(d.pos(a), Some(vec2(10.0, 10.0)));
    }

    #[test]
    fn nested_transactions_close_on_outermost_end_and_block_undo() {
        let mut d = doc();
        let mut h = History::new();
        let a = d.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        h.begin();
        h.begin();
        h.execute(
            &mut d,
            Command::SetFlag {
                uid: a,
                flag: Flag::Muted,
                old: false,
                new: true,
            },
        );
        h.end();
        assert!(h.in_transaction() && !h.can_undo());
        assert!(!h.undo(&mut d));
        h.execute(
            &mut d,
            Command::Resize {
                uid: a,
                old: (180.0, 200.0),
                new: (240.0, 200.0),
            },
        );
        h.end();
        assert!(!h.in_transaction());
        assert_eq!(h.undo_len(), 1);
        assert!(matches!(h.undo.last(), Some(Command::Transaction(v)) if v.len() == 2));
        h.undo(&mut d);
        assert!(!d.data(a).unwrap().muted);
        assert_eq!(d.size(a), Some((180.0, 200.0)));
    }

    #[test]
    fn new_command_clears_redo_and_limit_drops_oldest() {
        let mut d = doc();
        let mut h = History::new();
        h.set_limit(2);
        let a = d.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        for i in 1..=3 {
            let o = vec2((i - 1) as f32, 0.0);
            let n = vec2(i as f32, 0.0);
            h.execute(&mut d, Command::MoveNodes(vec![(a, o, n)]));
        }
        assert_eq!(h.undo_len(), 2);
        h.undo(&mut d);
        assert!(h.can_redo());
        h.execute(
            &mut d,
            Command::SetFlag {
                uid: a,
                flag: Flag::Collapsed,
                old: false,
                new: true,
            },
        );
        assert!(!h.can_redo());
    }

    #[test]
    fn invert_is_an_involution() {
        let c = Command::SetValue {
            uid: 1,
            input: 0,
            old: Value::Int(1),
            new: Value::Int(2),
        };
        assert_eq!(c.invert().invert(), c);
        let t = Command::Transaction(vec![
            c.clone(),
            Command::MoveNodes(vec![(1, vec2(0.0, 0.0), vec2(1.0, 1.0))]),
        ]);
        assert_eq!(t.invert().invert(), t);
    }
}
