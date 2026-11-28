//! Page items (PRD 5.6, blank canvas): text boxes, pictures, shapes, lines
//! and tables placed on a page as real page content, not annotations.
//!
//! A page's items are a list (its order is the drawing order) held per page
//! slot like headers and footers, so every change is one undo step. They are
//! drawn in one content stream after the page's own. When saved, the page
//! keeps the list in /PieceInfo /Edits and the stream is marked as edits'
//! own (/EditsItems), so that edits can take it out and show the items as
//! items again when the file is reopened. Other readers see plain content.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use edits_text::{Align, FontId, Style};
use serde::{Deserialize, Serialize};

use super::decor::Frame;
use super::pages::{Out, PageSrc};
use super::{EditError, Session, font_objects, image_objects};
use crate::image::PdfImage;
use crate::pdf::object::real;
use crate::pdf::{Dict, Obj, PdfFile, Ref, Source};

pub type Rgb = [f32; 3];

/// A text block: layout, left, top, width, colour.
type Block = (edits_text::Layout, f32, f32, f32, Rgb);
type BlockRef<'a> = (&'a edits_text::Layout, f32, f32, f32, Rgb);

/// How text in an item is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    /// Font family; empty for the bundled IBM Plex Sans Arabic.
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    pub size: f32,
    pub color: Rgb,
    /// "start", "left", "center" or "right".
    pub align: String,
    /// The paragraph direction: None from the text, Some(true) right to left.
    pub rtl: Option<bool>,
    pub line_height: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font: String::new(),
            bold: false,
            italic: false,
            size: 12.0,
            color: [0.0; 3],
            align: "start".into(),
            rtl: None,
            line_height: 1.3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextItem {
    pub text: String,
    pub style: TextStyle,
    pub fill: Option<Rgb>,
    pub border: Option<Rgb>,
    pub border_width: f32,
    /// Space between the box's edge and the text, in points.
    pub padding: f32,
}

impl Default for TextItem {
    fn default() -> Self {
        TextItem {
            text: String::new(),
            style: TextStyle::default(),
            fill: None,
            border: None,
            border_width: 1.0,
            padding: 4.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableItem {
    /// Column widths in points, left to right as shown (right to left
    /// tables list their first column last: `rtl`).
    pub widths: Vec<f32>,
    /// Smallest row heights in points; rows grow to fit their text.
    pub heights: Vec<f32>,
    /// Rows of cells, in reading order within a row.
    pub cells: Vec<Vec<String>>,
    pub style: TextStyle,
    /// The first column is at the right.
    pub rtl: bool,
    pub border: Rgb,
    pub border_width: f32,
    /// The first row in bold on this colour.
    pub header: Option<Rgb>,
    pub padding: f32,
}

impl Default for TableItem {
    fn default() -> Self {
        TableItem {
            widths: vec![80.0; 3],
            heights: vec![20.0; 3],
            cells: vec![vec![String::new(); 3]; 3],
            style: TextStyle { size: 10.0, ..TextStyle::default() },
            rtl: false,
            border: [0.0; 3],
            border_width: 0.75,
            header: Some([0.9, 0.9, 0.9]),
            padding: 3.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ItemKind {
    Text(TextItem),
    /// Picture `image` of the session's pictures, stretched to the box.
    Image {
        image: u32,
    },
    Shape {
        ellipse: bool,
        stroke: Option<Rgb>,
        width: f32,
        fill: Option<Rgb>,
    },
    /// A line across the box: from the shown top-left to bottom-right, or
    /// (`flip`) bottom-left to top-right; `reverse` swaps its ends (the
    /// arrow head is at the end).
    Line {
        flip: bool,
        reverse: bool,
        color: Rgb,
        width: f32,
        arrow: bool,
    },
    Table(TableItem),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Unique in the session.
    pub id: u32,
    /// Box in user space: left, bottom, right, top.
    pub rect: [f32; 4],
    /// Quarter turns clockwise the item is drawn at, so it reads upright on
    /// a page shown turned (its /Rotate when the item was placed).
    #[serde(default)]
    pub turn: u8,
    /// Items with the same group move and are chosen together.
    #[serde(default)]
    pub group: Option<u32>,
    pub kind: ItemKind,
}

/// Every installed font edits can embed, read once.
fn system_fonts() -> &'static [edits_text::system::FontFace] {
    static F: OnceLock<Vec<edits_text::system::FontFace>> = OnceLock::new();
    F.get_or_init(edits_text::system::fonts)
}

/// The installed font families, for the interface: (name, has Arabic).
pub fn font_families() -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for f in system_fonts() {
        match out.last_mut() {
            Some(last) if last.0 == f.family => last.1 |= f.arabic,
            _ => out.push((f.family.clone(), f.arabic)),
        }
    }
    out
}

impl TextStyle {
    /// The font to set this text in (Plex if the family is not installed).
    pub fn font_id(&self) -> FontId {
        let plex = if self.bold { edits_text::DEFAULT_BOLD } else { edits_text::DEFAULT_FONT };
        if self.font.is_empty() {
            return plex;
        }
        edits_text::system::pick(system_fonts(), &self.font, self.bold, self.italic)
            .and_then(|f| edits_text::load(&f.path, f.index).ok())
            .unwrap_or(plex)
    }

    fn style(&self) -> Style {
        Style {
            font: self.font_id(),
            size: self.size.clamp(1.0, 400.0),
            line_height: self.line_height.clamp(0.8, 3.0),
            align: match self.align.as_str() {
                "left" => Align::Left,
                "center" => Align::Center,
                "right" => Align::Right,
                _ => Align::Start,
            },
            rtl: self.rtl,
        }
    }

    fn color(&self) -> Rgb {
        self.color.map(|c| c.clamp(0.0, 1.0))
    }
}

impl Item {
    fn frame(&self) -> Frame {
        Frame { bx: self.rect, rotate: (self.turn % 4) as i64 * 90 }
    }

    /// Width and height as shown.
    pub fn shown_size(&self) -> (f32, f32) {
        let (w, h) = ((self.rect[2] - self.rect[0]).abs(), (self.rect[3] - self.rect[1]).abs());
        if self.turn.is_multiple_of(2) { (w, h) } else { (h, w) }
    }

    /// Makes the shown height `h`, keeping the shown top edge where it is.
    fn set_shown_height(&mut self, h: f32) {
        let r = &mut self.rect;
        match self.turn % 4 {
            0 => r[1] = r[3] - h,
            1 => r[2] = r[0] + h,
            2 => r[3] = r[1] + h,
            _ => r[0] = r[2] - h,
        }
    }

    /// Makes the shown width `w`, keeping the shown left edge where it is.
    fn set_shown_width(&mut self, w: f32) {
        let r = &mut self.rect;
        match self.turn % 4 {
            0 => r[2] = r[0] + w,
            1 => r[1] = r[3] - w,
            2 => r[0] = r[2] - w,
            _ => r[3] = r[1] + w,
        }
    }

    /// Sorts the box's corners and grows text boxes and tables to fit their
    /// text (downwards, as shown).
    pub fn fit(&mut self) {
        let r = self.rect;
        self.rect = [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])];
        match &self.kind {
            ItemKind::Text(t) => {
                let (w, h) = self.shown_size();
                let lay = edits_text::layout_with(&t.text, &t.style.style(), (w - 2.0 * t.padding).max(1.0));
                let need = lay.height() + 2.0 * t.padding;
                if need > h {
                    self.set_shown_height(need);
                }
            }
            ItemKind::Table(t) => {
                let (w, h) = table_size(t);
                self.set_shown_width(w);
                self.set_shown_height(h);
            }
            _ => {}
        }
    }
}

/// A table's row heights, grown to fit the text of their cells.
fn row_heights(t: &TableItem) -> Vec<f32> {
    let style = cell_style(t, &t.style);
    (0..t.cells.len())
        .map(|r| {
            let min = t.heights.get(r).copied().unwrap_or(20.0).max(4.0);
            let mut s = style;
            if r == 0 && t.header.is_some() {
                s.font = TextStyle { bold: true, ..t.style.clone() }.font_id();
            }
            t.cells[r]
                .iter()
                .enumerate()
                .map(|(c, text)| {
                    let w = t.widths.get(c).copied().unwrap_or(40.0) - 2.0 * t.padding;
                    edits_text::layout_with(text, &s, w.max(1.0)).height() + 2.0 * t.padding
                })
                .fold(min, f32::max)
        })
        .collect()
}

/// How a table's cells are set: in a right-to-left table they read right
/// to left (numbers and Latin words align right too) unless the style says.
fn cell_style(t: &TableItem, st: &TextStyle) -> Style {
    let mut s = st.style();
    if t.rtl && s.rtl.is_none() {
        s.rtl = Some(true);
    }
    s
}

fn table_size(t: &TableItem) -> (f32, f32) {
    (t.widths.iter().sum::<f32>().max(4.0), row_heights(t).iter().sum::<f32>().max(4.0))
}

/// Keeps a table's rows and columns in step: every row as many cells as
/// there are columns, and a height per row.
pub fn tidy_table(t: &mut TableItem) {
    let cols = t.widths.len().max(1);
    if t.widths.is_empty() {
        t.widths.push(80.0);
    }
    for w in &mut t.widths {
        *w = w.clamp(8.0, 2000.0);
    }
    if t.cells.is_empty() {
        t.cells.push(vec![String::new(); cols]);
    }
    for row in &mut t.cells {
        row.resize(cols, String::new());
    }
    t.heights.resize(t.cells.len(), 20.0);
}

fn rg(c: Rgb) -> String {
    format!("{} {} {}", real(c[0] as f64), real(c[1] as f64), real(c[2] as f64))
}

fn cm(m: [f32; 6]) -> String {
    m.iter().map(|v| real(*v as f64)).collect::<Vec<_>>().join(" ") + " cm\n"
}

/// A rectangle path in the item's shown coordinates.
fn re(x: f32, y: f32, w: f32, h: f32) -> String {
    format!("{} {} {} {} re\n", real(x as f64), real(y as f64), real(w as f64), real(h as f64))
}

/// An ellipse path filling a w x h box (four Bézier arcs).
fn ellipse(w: f32, h: f32) -> String {
    const K: f32 = 0.552_284_8;
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (rx, ry);
    let p = |v: f32| real(v as f64);
    let mut s = format!("{} {} m\n", p(cx + rx), p(cy));
    for (a, b, c, d, e, f) in [
        (cx + rx, cy + K * ry, cx + K * rx, cy + ry, cx, cy + ry),
        (cx - K * rx, cy + ry, cx - rx, cy + K * ry, cx - rx, cy),
        (cx - rx, cy - K * ry, cx - K * rx, cy - ry, cx, cy - ry),
        (cx + K * rx, cy - ry, cx + rx, cy - K * ry, cx + rx, cy),
    ] {
        s += &format!("{} {} {} {} {} {} c\n", p(a), p(b), p(c), p(d), p(e), p(f));
    }
    s
}

/// Fill and stroke operators for a path, or "n" (nothing) if neither.
fn paint(fill: Option<Rgb>, stroke: Option<Rgb>, width: f32) -> String {
    let mut s = String::new();
    if let Some(f) = fill {
        s += &format!("{} rg\n", rg(f));
    }
    if let Some(c) = stroke {
        s += &format!("{} RG\n{} w\n", rg(c), real(width.max(0.0) as f64));
    }
    s += match (fill.is_some(), stroke.is_some() && width > 0.0) {
        (true, true) => "B\n",
        (true, false) => "f\n",
        (false, true) => "S\n",
        (false, false) => "n\n",
    };
    s
}

/// One page's items as content, with the text written in two passes (all
/// pages' text shares fonts): `parts` are the text blocks, and `ops` holds
/// the operators with `None` where each part's text goes.
pub(super) struct Drawn {
    pub parts: Vec<Vec<Block>>,
    pub ops: Vec<Option<String>>,
    /// Pictures used: (resource name, picture index).
    pub images: Vec<(String, u32)>,
}

/// Lays out `items` for drawing.
pub(super) fn draw(items: &[Item]) -> Drawn {
    let mut d = Drawn { parts: Vec::new(), ops: vec![Some("Q\nq\n".into())], images: Vec::new() };
    let text = |d: &mut Drawn, blocks: Vec<Block>| {
        d.parts.push(blocks);
        d.ops.push(None);
    };
    for it in items {
        let (w, h) = it.shown_size();
        let mut s = String::from("q\n");
        s += &cm(it.frame().matrix());
        match &it.kind {
            ItemKind::Text(t) => {
                if t.fill.is_some() || t.border.is_some() {
                    s += &re(0.0, 0.0, w, h);
                    s += &paint(t.fill, t.border, t.border_width);
                }
                d.ops.push(Some(s));
                let lay = edits_text::layout_with(&t.text, &t.style.style(), (w - 2.0 * t.padding).max(1.0));
                text(&mut d, vec![(lay, t.padding, h - t.padding, w - 2.0 * t.padding, t.style.color())]);
                s = String::new();
            }
            ItemKind::Image { image } => {
                let name = format!("EditsIm{}", it.id);
                s += &format!("{} 0 0 {} 0 0 cm /{name} Do\n", real(w as f64), real(h as f64));
                d.images.push((name, *image));
            }
            ItemKind::Shape { ellipse: e, stroke, width, fill } => {
                s += &if *e { ellipse(w, h) } else { re(0.0, 0.0, w, h) };
                s += &paint(*fill, *stroke, *width);
            }
            ItemKind::Line { flip, reverse, color, width, arrow } => {
                let (mut a, mut b) = if *flip { ((0.0, 0.0), (w, h)) } else { ((0.0, h), (w, 0.0)) };
                if *reverse {
                    std::mem::swap(&mut a, &mut b);
                }
                let p = |v: f32| real(v as f64);
                s += &format!(
                    "{} RG\n{} w\n1 J\n{} {} m\n{} {} l\nS\n",
                    rg(*color),
                    p(*width),
                    p(a.0),
                    p(a.1),
                    p(b.0),
                    p(b.1)
                );
                if *arrow {
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let len = dx.hypot(dy).max(0.001);
                    let (ux, uy) = (dx / len, dy / len);
                    let head = (4.0 * width).max(6.0);
                    let (bx, by) = (b.0 - ux * head, b.1 - uy * head);
                    let (px, py) = (-uy * head * 0.45, ux * head * 0.45);
                    s += &format!(
                        "{} rg\n{} {} m\n{} {} l\n{} {} l\nh f\n",
                        rg(*color),
                        p(b.0),
                        p(b.1),
                        p(bx + px),
                        p(by + py),
                        p(bx - px),
                        p(by - py)
                    );
                }
            }
            ItemKind::Table(t) => {
                let heights = row_heights(t);
                let total_h: f32 = heights.iter().sum();
                // Column left edges as shown.
                let order: Vec<usize> =
                    if t.rtl { (0..t.widths.len()).rev().collect() } else { (0..t.widths.len()).collect() };
                let mut lefts = vec![0.0; t.widths.len()];
                let mut x = 0.0;
                for &c in &order {
                    lefts[c] = x;
                    x += t.widths[c];
                }
                let total_w = x;
                if let Some(fill) = t.header
                    && let Some(h0) = heights.first()
                {
                    s += &re(0.0, total_h - h0, total_w, *h0);
                    s += &paint(Some(fill), None, 0.0);
                }
                // Grid.
                if t.border_width > 0.0 {
                    let mut g = String::new();
                    let mut y = total_h;
                    g += &format!("0 {} m {} {} l\n", real(y as f64), real(total_w as f64), real(y as f64));
                    for hh in &heights {
                        y -= hh;
                        g += &format!("0 {} m {} {} l\n", real(y as f64), real(total_w as f64), real(y as f64));
                    }
                    let mut x = 0.0;
                    g += &format!("0 0 m 0 {} l\n", real(total_h as f64));
                    for &c in &order {
                        x += t.widths[c];
                        g += &format!("{} 0 m {} {} l\n", real(x as f64), real(x as f64), real(total_h as f64));
                    }
                    s += &format!("{} RG\n{} w\n{g}S\n", rg(t.border), real(t.border_width as f64));
                }
                d.ops.push(Some(s));
                let mut blocks = Vec::new();
                let mut top = total_h;
                for (r, row) in t.cells.iter().enumerate() {
                    let mut st = t.style.clone();
                    if r == 0 && t.header.is_some() {
                        st.bold = true;
                    }
                    let style = cell_style(t, &st);
                    for (c, cell) in row.iter().enumerate() {
                        let Some(cw) = t.widths.get(c) else { continue };
                        let inner = (cw - 2.0 * t.padding).max(1.0);
                        let lay = edits_text::layout_with(cell, &style, inner);
                        blocks.push((lay, lefts[c] + t.padding, top - t.padding, inner, st.color()));
                    }
                    top -= heights.get(r).copied().unwrap_or(0.0);
                }
                text(&mut d, blocks);
                s = String::new();
            }
        }
        s += "Q\n";
        d.ops.push(Some(s));
    }
    d.ops.push(Some("Q\n".into()));
    d
}

/// The resource-name prefix of item text fonts.
pub(super) const FONT_RES: &str = "EditsT";

impl Session {
    /// Writes the items of the pages in `pages` (where each page dictionary
    /// is in `out.objects`, and its items) into those pages.
    pub(super) fn draw_items(&self, pages: &[(usize, Arc<Vec<Item>>)], out: &mut Out) {
        let drawn: Vec<Drawn> = pages.iter().map(|(_, items)| draw(items)).collect();
        let owned: Vec<Vec<BlockRef>> = drawn
            .iter()
            .flat_map(|d| d.parts.iter().map(|p| p.iter().map(|b| (&b.0, b.1, b.2, b.3, b.4)).collect()))
            .collect();
        let refs: Vec<&edits_text::Blocks> = owned.iter().map(|p| p.as_slice()).collect();
        let (texts, fonts) = edits_text::pdf_text_fonts(&refs, FONT_RES);
        let fonts: Vec<(String, Ref)> =
            fonts.iter().map(|(name, f)| (name.clone(), font_objects(f, &mut |o| out.add(o)))).collect();
        let mut texts = texts.into_iter();
        let mut open = Dict::default();
        open.set(b"EditsOpen", Obj::Bool(true));
        let open = out.add(Obj::Stream(open, b"q\n".to_vec()));
        let mut image_refs: HashMap<u32, Ref> = HashMap::new();
        for ((at, items), d) in pages.iter().zip(&drawn) {
            let mut content = String::new();
            for op in &d.ops {
                match op {
                    Some(s) => content += s,
                    None => content += &texts.next().unwrap_or_default(),
                }
            }
            let mut sd = Dict::default();
            sd.set(b"EditsItems", Obj::Bool(true));
            sd.set(b"Filter", Obj::name("FlateDecode"));
            let stream = out.add(Obj::Stream(sd, super::deflate(content.as_bytes())));
            let mut xobjects = Vec::new();
            for (name, n) in &d.images {
                let r = match image_refs.get(n) {
                    Some(r) => *r,
                    None => {
                        let Some(img) = self.images.get(*n as usize) else { continue };
                        let r = image_objects(img, &mut |o| out.add(o));
                        image_refs.insert(*n, r);
                        r
                    }
                };
                xobjects.push((name.clone(), r));
            }
            let json = serde_json::to_string(items.as_ref()).unwrap_or_default();
            let Obj::Dict(dict) = &mut out.objects[*at].1 else { continue };
            let mut contents = vec![Obj::Ref(open)];
            match dict.get(b"Contents") {
                Some(Obj::Array(a)) => contents.extend(a.iter().cloned()),
                Some(c @ Obj::Ref(_)) => contents.push(c.clone()),
                _ => {}
            }
            contents.push(Obj::Ref(stream));
            dict.set(b"Contents", Obj::Array(contents));
            let mut res = dict.get(b"Resources").and_then(Obj::as_dict).cloned().unwrap_or_default();
            if !fonts.is_empty() {
                let mut fd = res.get(b"Font").and_then(Obj::as_dict).cloned().unwrap_or_default();
                for (name, r) in &fonts {
                    fd.set(name.as_bytes(), Obj::Ref(*r));
                }
                res.set(b"Font", Obj::Dict(fd));
            }
            if !xobjects.is_empty() {
                let mut xd = res.get(b"XObject").and_then(Obj::as_dict).cloned().unwrap_or_default();
                for (name, r) in xobjects {
                    xd.set(name.as_bytes(), Obj::Ref(r));
                }
                res.set(b"XObject", Obj::Dict(xd));
            }
            dict.set(b"Resources", Obj::Dict(res));
            let mut edits = Dict::default();
            edits.set(b"LastModified", Obj::Str(crate::model::pdf_date_now().into_bytes()));
            edits.set(b"Private", Obj::Str(json.into_bytes()));
            let mut piece = dict.get(b"PieceInfo").and_then(Obj::as_dict).cloned().unwrap_or_default();
            piece.set(b"Edits", Obj::Dict(edits));
            dict.set(b"PieceInfo", Obj::Dict(piece));
        }
    }

    /// The items on the page at `index`: those changed in this session, or
    /// those saved in the file by edits.
    pub fn items(&mut self, index: u32) -> Vec<Item> {
        let Some(slot) = self.order().get(index as usize).cloned() else { return Vec::new() };
        if let Some(k) = slot.items {
            return self.item_lists.get(k as usize).map(|l| l.as_ref().clone()).unwrap_or_default();
        }
        self.saved_items(&slot.src).unwrap_or_default()
    }

    /// Items saved in the base file on a page, read once; their pictures
    /// join the session's pictures.
    fn saved_items(&mut self, src: &PageSrc) -> Option<Vec<Item>> {
        let PageSrc::Base(i) = src else { return None };
        if let Some(l) = self.saved.get(i) {
            return l.clone();
        }
        let found = read_saved(&self.base, *i).map(|(mut items, pictures)| {
            for it in &mut items {
                if let ItemKind::Image { image } = &mut it.kind {
                    match pictures.get(&it.id) {
                        Some(img) => {
                            self.images.push(Arc::new(img.clone()));
                            *image = self.images.len() as u32 - 1;
                        }
                        None => *image = u32::MAX,
                    }
                }
                self.next_item = self.next_item.max(it.id + 1);
            }
            items.retain(|it| !matches!(it.kind, ItemKind::Image { image: u32::MAX }));
            items
        });
        self.saved.insert(*i, found.clone());
        found
    }

    /// Replaces the items on the page at `index` (one undo step). Ids of
    /// new items (0) are given here; boxes are fitted to their text.
    pub fn set_items(&mut self, index: u32, mut items: Vec<Item>) -> Result<Vec<Item>, EditError> {
        self.checked(&[index])?;
        // Pictures and saved items are known before the list changes.
        let _ = self.items(index);
        for it in &mut items {
            if it.id == 0 {
                self.next_item += 1;
                it.id = self.next_item;
            }
            if let ItemKind::Table(t) = &mut it.kind {
                tidy_table(t);
            }
            it.fit();
        }
        self.item_lists.push(Arc::new(items.clone()));
        let mut order = self.order().to_vec();
        order[index as usize].items = Some(self.item_lists.len() as u32 - 1);
        self.set_order(order)?;
        Ok(items)
    }

    /// The margins the document was made with (left, top, right, bottom,
    /// points), for guides; None for other documents.
    pub fn margins(&self) -> Option<[f32; 4]> {
        let root = self.base.resolve(self.base.trailer.get(b"Root")?).ok()?;
        let m = self.base.resolve(root.as_dict()?.get(b"EditsMargins")?).ok()?;
        let n = m.numbers()?;
        (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
    }

    /// Adds a picture for picture items; returns its number.
    pub fn add_picture(&mut self, img: PdfImage) -> u32 {
        self.images.push(Arc::new(img));
        self.images.len() as u32 - 1
    }

    /// A picture's size in pixels.
    pub fn picture_size(&self, n: u32) -> Option<(u32, u32)> {
        self.images.get(n as usize).map(|i| (i.width, i.height))
    }
}

/// The items edits saved on base page `i`, and their pictures by item id.
fn read_saved<S: Source>(f: &PdfFile<S>, i: u32) -> Option<(Vec<Item>, HashMap<u32, PdfImage>)> {
    let page = *f.pages().ok()?.get(i as usize)?;
    let d = super::pages::page_dict(f, page).ok()?;
    let piece = f.resolve(d.get(b"PieceInfo")?).ok()?;
    let edits = f.resolve(piece.as_dict()?.get(b"Edits")?).ok()?;
    let private = f.resolve(edits.as_dict()?.get(b"Private")?).ok()?;
    let Obj::Str(json) = private else { return None };
    let items: Vec<Item> = serde_json::from_slice(&json).ok()?;
    // Only when the items' own stream is still there: otherwise another
    // program rewrote the page, and its content is all there is.
    if !contents_of(f, &d).iter().any(|(_, sd)| sd.get(b"EditsItems").is_some()) {
        return None;
    }
    let xobjects = d
        .get(b"Resources")
        .and_then(|r| f.resolve(r).ok())
        .and_then(|r| r.as_dict().and_then(|r| r.get(b"XObject")).and_then(|x| f.resolve(x).ok()));
    let mut pictures = HashMap::new();
    for it in &items {
        if !matches!(it.kind, ItemKind::Image { .. }) {
            continue;
        }
        let name = format!("EditsIm{}", it.id);
        let img = xobjects
            .as_ref()
            .and_then(|x| x.as_dict()?.get(name.as_bytes()).and_then(Obj::as_ref))
            .and_then(|r| picture_from(f, r));
        if let Some(img) = img {
            pictures.insert(it.id, img);
        }
    }
    Some((items, pictures))
}

/// The page's content streams: reference and stream dictionary.
fn contents_of<S: Source>(f: &PdfFile<S>, d: &Dict) -> Vec<(Ref, Dict)> {
    let refs: Vec<Ref> = match d.get(b"Contents") {
        Some(Obj::Ref(r)) => match f.get(*r) {
            Ok(Obj::Array(a)) => a.iter().filter_map(Obj::as_ref).collect(),
            _ => vec![*r],
        },
        Some(Obj::Array(a)) => a.iter().filter_map(Obj::as_ref).collect(),
        _ => Vec::new(),
    };
    refs.into_iter()
        .filter_map(|r| match f.get(r) {
            Ok(Obj::Stream(sd, _)) => Some((r, sd)),
            _ => None,
        })
        .collect()
}

/// Takes edits' item streams (and the q that opens them) out of a page
/// dictionary that is being written again with new items.
pub(super) fn strip_saved<S: Source>(f: &PdfFile<S>, d: &mut Dict) {
    let streams = contents_of(f, d);
    if !streams.iter().any(|(_, sd)| sd.get(b"EditsItems").is_some()) {
        return;
    }
    // Drop one opening q per items stream, so q and Q stay balanced.
    let mut items = streams.iter().filter(|(_, sd)| sd.get(b"EditsItems").is_some()).count();
    let keep: Vec<Obj> = streams
        .iter()
        .filter(|(_, sd)| {
            if sd.get(b"EditsItems").is_some() {
                return false;
            }
            if sd.get(b"EditsOpen").is_some() && items > 0 {
                items -= 1;
                return false;
            }
            true
        })
        .map(|(r, _)| Obj::Ref(*r))
        .collect();
    d.set(b"Contents", Obj::Array(keep));
    if let Some(Obj::Dict(p)) = d.get(b"PieceInfo").cloned() {
        let mut p = p;
        p.remove(b"Edits");
        if p.0.is_empty() {
            d.remove(b"PieceInfo");
        } else {
            d.set(b"PieceInfo", Obj::Dict(p));
        }
    }
}

/// A picture as edits wrote it (JPEG or Flate, 8 bits, with a soft mask).
fn picture_from<S: Source>(f: &PdfFile<S>, r: Ref) -> Option<PdfImage> {
    let Ok(Obj::Stream(d, data)) = f.get(r) else { return None };
    let int = |k: &[u8]| d.get(k).and_then(Obj::as_int);
    let alpha = match d.get(b"SMask").and_then(Obj::as_ref).map(|m| f.get(m)) {
        Some(Ok(Obj::Stream(_, a))) => Some(a),
        _ => None,
    };
    Some(PdfImage {
        width: int(b"Width")? as u32,
        height: int(b"Height")? as u32,
        color_space: String::from_utf8_lossy(d.name(b"ColorSpace")?).into_owned(),
        filter: String::from_utf8_lossy(d.name(b"Filter")?).into_owned(),
        data,
        alpha,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::pages::tests::{pages_pdf, temp};

    fn text(id: u32, s: &str, rect: [f32; 4]) -> Item {
        Item {
            id,
            rect,
            turn: 0,
            group: None,
            kind: ItemKind::Text(TextItem { text: s.into(), ..TextItem::default() }),
        }
    }

    /// The page's content streams, decoded, and its dictionary.
    fn page_content(bytes: Vec<u8>, i: usize) -> (Vec<String>, Dict) {
        let f = PdfFile::open(bytes).unwrap();
        let pages = f.pages().unwrap();
        let d = crate::session::pages::page_dict(&f, pages[i]).unwrap();
        let streams = contents_of(&f, &d)
            .into_iter()
            .map(|(r, sd)| {
                let Obj::Stream(_, raw) = f.get(r).unwrap() else { panic!() };
                String::from_utf8_lossy(&crate::pdf::file::decode(&sd, &raw).unwrap()).into_owned()
            })
            .collect();
        (streams, d)
    }

    #[test]
    fn items_are_page_content_and_come_back_after_saving() {
        let path = temp("items.pdf", &pages_pdf(3));
        let mut s = Session::open(&path).unwrap();
        let png = {
            let mut out = Vec::new();
            {
                let mut e = png::Encoder::new(&mut out, 2, 2);
                e.set_color(png::ColorType::Rgb);
                e.write_header().unwrap().write_image_data(&[200u8; 12]).unwrap();
            }
            PdfImage::from_bytes(&out).unwrap()
        };
        let pic = s.add_picture(png);
        let items = vec![
            text(0, "فاتورة Invoice", [50.0, 700.0, 250.0, 710.0]),
            Item {
                id: 0,
                rect: [10.0, 10.0, 60.0, 40.0],
                turn: 0,
                group: Some(1),
                kind: ItemKind::Shape { ellipse: true, stroke: Some([1.0, 0.0, 0.0]), width: 2.0, fill: None },
            },
            Item {
                id: 0,
                rect: [70.0, 10.0, 90.0, 30.0],
                turn: 0,
                group: Some(1),
                kind: ItemKind::Image { image: pic },
            },
            Item {
                id: 0,
                rect: [10.0, 100.0, 100.0, 100.0],
                turn: 0,
                group: None,
                kind: ItemKind::Line { flip: false, reverse: false, color: [0.0; 3], width: 1.0, arrow: true },
            },
            Item {
                id: 0,
                rect: [10.0, 200.0, 10.0, 200.0],
                turn: 0,
                group: None,
                kind: ItemKind::Table(TableItem::default()),
            },
        ];
        let set = s.set_items(1, items).unwrap();
        assert!(set.iter().all(|i| i.id > 0), "ids given");
        assert!(set[0].rect[3] - set[0].rect[1] > 10.0, "the text box grew to fit");
        let ItemKind::Table(t) = &set[4].kind else { panic!() };
        assert_eq!((set[4].rect[2] - set[4].rect[0]), t.widths.iter().sum::<f32>(), "a table is its columns wide");
        assert!(!s.take_layout_changed(), "content only");
        assert_eq!(s.take_redraw(), vec![1]);

        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&s.tail().unwrap());
        let (streams, d) = page_content(bytes, 1);
        let items_stream = streams.last().unwrap();
        assert!(
            items_stream.starts_with("Q\nq\n") && items_stream.contains(" c\n") && items_stream.contains("/EditsIm")
        );
        assert!(items_stream.contains("/EditsT") && items_stream.contains(" re\n"), "text and table");
        assert!(d.get(b"PieceInfo").is_some());

        // Saved, then opened again: the items are items again.
        s.save(&path).unwrap();
        let mut s = Session::open(&path).unwrap();
        let back = s.items(1);
        assert_eq!(back.len(), 5);
        assert_eq!(back[0].kind, set[0].kind);
        assert!(matches!(back[2].kind, ItemKind::Image { .. }), "the picture came back");
        assert!(s.items(0).is_empty());

        // Changing them writes the page again without the old stream.
        let mut fewer = back.clone();
        fewer.truncate(1);
        s.set_items(1, fewer).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&s.tail().unwrap());
        let (streams, _) = page_content(bytes, 1);
        let ours: Vec<&String> = streams.iter().filter(|c| c.contains("/EditsT")).collect();
        assert_eq!(ours.len(), 1, "one items stream");
        assert!(!ours[0].contains("/EditsIm"), "the removed picture is gone");
        let opens = streams.iter().filter(|c| c.trim() == "q").count();
        let closes = streams.iter().filter(|c| c.starts_with("Q\nq\n")).count();
        assert_eq!(opens, closes, "q and Q stay balanced");

        // Undo is a content change too.
        s.undo().unwrap();
        assert!(!s.take_layout_changed());
        assert_eq!(s.take_redraw(), vec![1]);
        assert_eq!(s.items(1).len(), 5);
    }

    #[test]
    fn turned_items_keep_their_top_when_they_grow() {
        for turn in 0..4u8 {
            let mut it = text(1, "a\nb\nc\nd", [100.0, 100.0, 200.0, 200.0]);
            it.turn = turn;
            if turn % 2 == 1 {
                it.rect = [100.0, 100.0, 150.0, 300.0];
            }
            let before = it.rect;
            it.fit();
            let (w, _) = it.shown_size();
            assert!(w > 40.0);
            // The edge that is the top as shown has not moved.
            let same = match turn {
                0 => it.rect[3] == before[3],
                1 => it.rect[0] == before[0],
                2 => it.rect[1] == before[1],
                _ => it.rect[2] == before[2],
            };
            assert!(same, "turn {turn}: {before:?} -> {:?}", it.rect);
        }
    }
}
