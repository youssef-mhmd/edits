//! A document's edits and their history (PRD 8.4): every change is a
//! command that can be undone and redone.
//!
//! Annotations live in two places. Here, as data, which is what gets saved.
//! And in PDFium's copy of the document, so pages render with them (the
//! "layer"). The layer only ever grows: a deleted annotation is hidden, not
//! removed, so PDFium's annotation indices stay valid and undo just shows it
//! again. Saving writes the data into the file, the document is reopened,
//! and the layer starts again from the file.

use std::collections::{BTreeMap, HashMap};

use crate::model::{Annot, AnnotId, ObjRef};
use crate::save::{PageChange, SaveOutcome};

/// What the PDFium copy of the document must do to match the data.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerOp {
    /// Create the annotation; report its index with [`Edits::set_layer`].
    Create { id: AnnotId, annot: Annot },
    /// Hide or show the annotation at `index` on `page`.
    SetHidden { page: u32, index: u32, hidden: bool },
}

#[derive(Debug, Clone)]
struct Entry {
    annot: Annot,
    /// Part of the document now (false once deleted).
    alive: bool,
    /// Its object in the file as last saved, if it is there.
    in_file: Option<ObjRef>,
    /// Its index among the page's annotations in PDFium's copy.
    layer: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
enum Command {
    Add(AnnotId),
    Remove(AnnotId),
}

/// An annotation as the interface sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Listed {
    pub id: AnnotId,
    #[serde(flatten)]
    pub annot: Annot,
}

/// Undo and redo state, for the interface.
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub struct Status {
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Default)]
pub struct Edits {
    entries: BTreeMap<AnnotId, Entry>,
    undo: Vec<(u64, Command)>,
    redo: Vec<(u64, Command)>,
    next_id: AnnotId,
    next_seq: u64,
    /// History position at the last save (0: as opened).
    saved_at: u64,
}

impl Edits {
    pub fn new() -> Self {
        Self::default()
    }

    /// The history position: the sequence number of the last applied command.
    fn at(&self) -> u64 {
        self.undo.last().map_or(0, |(s, _)| *s)
    }

    pub fn status(&self) -> Status {
        Status { dirty: self.at() != self.saved_at, can_undo: !self.undo.is_empty(), can_redo: !self.redo.is_empty() }
    }

    fn push(&mut self, c: Command) {
        self.next_seq += 1;
        self.undo.push((self.next_seq, c));
        self.redo.clear();
    }

    /// Adds an annotation. Returns its id and what the layer must do.
    pub fn add(&mut self, annot: Annot) -> (AnnotId, Vec<LayerOp>) {
        self.next_id += 1;
        let id = self.next_id;
        self.entries.insert(id, Entry { annot, alive: false, in_file: None, layer: None });
        self.push(Command::Add(id));
        (id, self.show(id))
    }

    /// Deletes an annotation. Unknown or already deleted ids change nothing.
    pub fn remove(&mut self, id: AnnotId) -> Vec<LayerOp> {
        if !self.entries.get(&id).is_some_and(|e| e.alive) {
            return Vec::new();
        }
        self.push(Command::Remove(id));
        self.hide(id)
    }

    pub fn undo(&mut self) -> Vec<LayerOp> {
        let Some((seq, c)) = self.undo.pop() else { return Vec::new() };
        self.redo.push((seq, c));
        match c {
            Command::Add(id) => self.hide(id),
            Command::Remove(id) => self.show(id),
        }
    }

    pub fn redo(&mut self) -> Vec<LayerOp> {
        let Some((seq, c)) = self.redo.pop() else { return Vec::new() };
        self.undo.push((seq, c));
        match c {
            Command::Add(id) => self.show(id),
            Command::Remove(id) => self.hide(id),
        }
    }

    fn show(&mut self, id: AnnotId) -> Vec<LayerOp> {
        let Some(e) = self.entries.get_mut(&id) else { return Vec::new() };
        e.alive = true;
        match e.layer {
            Some(index) => vec![LayerOp::SetHidden { page: e.annot.page, index, hidden: false }],
            None => vec![LayerOp::Create { id, annot: e.annot.clone() }],
        }
    }

    fn hide(&mut self, id: AnnotId) -> Vec<LayerOp> {
        let Some(e) = self.entries.get_mut(&id) else { return Vec::new() };
        e.alive = false;
        match e.layer {
            Some(index) => vec![LayerOp::SetHidden { page: e.annot.page, index, hidden: true }],
            None => Vec::new(),
        }
    }

    /// Records where the layer put a created annotation.
    pub fn set_layer(&mut self, id: AnnotId, index: u32) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.layer = Some(index);
        }
    }

    /// Forgets a created annotation's layer index (its creation failed).
    pub fn clear_layer(&mut self, id: AnnotId) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.layer = None;
        }
    }

    /// The annotations on `page` that are part of the document now.
    pub fn on_page(&self, page: u32) -> Vec<Listed> {
        self.entries
            .iter()
            .filter(|(_, e)| e.alive && e.annot.page == page)
            .map(|(id, e)| Listed { id: *id, annot: e.annot.clone() })
            .collect()
    }

    /// What saving must change in the file, page by page.
    pub fn changes(&self) -> Vec<PageChange> {
        let mut pages: BTreeMap<u32, PageChange> = BTreeMap::new();
        for (id, e) in &self.entries {
            let page = e.annot.page;
            match (e.alive, e.in_file) {
                (true, None) => {
                    pages.entry(page).or_insert_with(|| PageChange::new(page)).add.push((*id, e.annot.clone()))
                }
                (false, Some(obj)) => pages.entry(page).or_insert_with(|| PageChange::new(page)).remove.push(obj),
                _ => {}
            }
        }
        pages.into_values().collect()
    }

    /// Takes in a successful save: what is in the file now, and where each
    /// annotation sits in its page's list once the document is reopened.
    pub fn saved(&mut self, outcome: &SaveOutcome) {
        let by_obj: HashMap<ObjRef, AnnotId> = self
            .entries
            .iter()
            .filter_map(|(id, e)| e.in_file.map(|o| (o, *id)))
            .chain(outcome.objs.iter().map(|(id, o)| (*o, *id)))
            .collect();
        for (id, e) in &mut self.entries {
            if !outcome.pages.contains_key(&e.annot.page) {
                continue;
            }
            // The page was rewritten; its layer is rebuilt from the file.
            e.layer = None;
            if e.alive {
                e.in_file = outcome.objs.get(id).copied().or(e.in_file);
            } else {
                e.in_file = None;
            }
        }
        for order in outcome.pages.values() {
            for (index, obj) in order.iter().enumerate() {
                if let Some(id) = obj.and_then(|o| by_obj.get(&o))
                    && let Some(e) = self.entries.get_mut(id)
                    && e.alive
                {
                    e.layer = Some(index as u32);
                }
            }
        }
        self.saved_at = self.at();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    fn hl(page: u32) -> Annot {
        Annot {
            page,
            kind: Kind::Highlight,
            rect: [0.0, 0.0, 10.0, 10.0],
            quads: vec![[0.0, 10.0, 10.0, 10.0, 0.0, 0.0, 10.0, 0.0]],
            color: [1.0, 1.0, 0.0],
            opacity: 0.4,
            contents: String::new(),
            author: String::new(),
            modified: String::new(),
        }
    }

    #[test]
    fn add_undo_redo_uses_the_layer() {
        let mut e = Edits::new();
        let (id, ops) = e.add(hl(0));
        assert!(matches!(ops[..], [LayerOp::Create { .. }]));
        e.set_layer(id, 3);
        assert!(e.status().dirty);
        assert_eq!(e.undo(), vec![LayerOp::SetHidden { page: 0, index: 3, hidden: true }]);
        assert!(!e.status().dirty, "undone back to the opened state");
        assert_eq!(e.redo(), vec![LayerOp::SetHidden { page: 0, index: 3, hidden: false }]);
        assert_eq!(e.on_page(0).len(), 1);
        assert_eq!(e.remove(id), vec![LayerOp::SetHidden { page: 0, index: 3, hidden: true }]);
        assert!(e.on_page(0).is_empty());
        assert!(e.remove(id).is_empty(), "deleting twice does nothing");
    }

    #[test]
    fn a_new_command_clears_redo() {
        let mut e = Edits::new();
        e.add(hl(0));
        e.undo();
        assert!(e.status().can_redo);
        e.add(hl(1));
        assert!(!e.status().can_redo);
    }

    #[test]
    fn save_then_delete_removes_from_file() {
        let mut e = Edits::new();
        let (a, _) = e.add(hl(2));
        let (b, _) = e.add(hl(2));
        let ch = e.changes();
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].add.len(), 2);
        let outcome = SaveOutcome {
            objs: [(a, (40, 0)), (b, (41, 0))].into(),
            pages: [(2, vec![Some((7, 0)), Some((40, 0)), Some((41, 0))])].into(),
        };
        e.saved(&outcome);
        assert!(!e.status().dirty);
        assert!(e.changes().is_empty());
        assert_eq!(e.remove(a), vec![LayerOp::SetHidden { page: 2, index: 1, hidden: true }]);
        let ch = e.changes();
        assert_eq!(ch[0].remove, vec![(40, 0)]);
        assert!(ch[0].add.is_empty());
        // Undo the delete after a second save: the annotation comes back as new.
        e.saved(&SaveOutcome { objs: HashMap::new(), pages: [(2, vec![Some((7, 0)), Some((41, 0))])].into() });
        assert_eq!(e.undo().len(), 1);
        assert!(matches!(e.changes()[0].add[..], [(id, _)] if id == a));
    }
}
