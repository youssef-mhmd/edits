//! The fonts installed on this computer that edits can embed: TrueType
//! outlines, and a licence (OS/2 fsType) that allows embedding in
//! documents. On Windows: the system font folder and the user's own.

use std::path::{Path, PathBuf};

/// One installed font face.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFace {
    /// Family name, as Word shows it (e.g. "Times New Roman").
    pub family: String,
    pub path: PathBuf,
    /// Index in a collection (.ttc), 0 otherwise.
    pub index: u32,
    pub bold: bool,
    pub italic: bool,
    /// Has the Arabic letters.
    pub arabic: bool,
}

/// True if the font's licence lets it be embedded in a document for
/// editing or printing (not "restricted", not "bitmap only").
pub fn embeddable(face: &ttf_parser::Face) -> bool {
    match face.permissions() {
        Some(ttf_parser::Permissions::Restricted) => false,
        _ => face.tables().os2.is_none_or(|os2| os2.is_outline_embedding_allowed()),
    }
}

fn dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(w) = std::env::var_os("WINDIR") {
        out.push(Path::new(&w).join("Fonts"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        out.push(Path::new(&l).join("Microsoft").join("Windows").join("Fonts"));
    }
    out
}

/// The family name: typographic family (name id 16) if there is one, else
/// the legacy family (1), in English where both are given.
fn family_of(face: &ttf_parser::Face) -> Option<String> {
    let pick = |id: u16| {
        let names: Vec<_> = face.names().into_iter().filter(|n| n.name_id == id && n.is_unicode()).collect();
        names.iter().find(|n| n.language_id == 0x0409).or(names.first()).and_then(|n| n.to_string())
    };
    pick(ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| pick(ttf_parser::name_id::FAMILY))
}

/// Reads one font file: each embeddable TrueType face in it.
fn read_file(path: &Path) -> Vec<FontFace> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "ttf" | "ttc") {
        return Vec::new();
    }
    let Ok(data) = std::fs::read(path) else { return Vec::new() };
    let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    let mut out = Vec::new();
    for index in 0..count {
        let Ok(face) = ttf_parser::Face::parse(&data, index) else { continue };
        if face.tables().glyf.is_none() || !embeddable(&face) {
            continue;
        }
        let Some(family) = family_of(&face) else { continue };
        let arabic =
            ['\u{0627}', '\u{0628}', '\u{0645}'].iter().all(|c| face.glyph_index(*c).is_some_and(|g| g.0 != 0));
        out.push(FontFace {
            family,
            path: path.to_path_buf(),
            index,
            bold: face.is_bold() || face.weight().to_number() >= 600,
            italic: face.is_italic() || face.is_oblique(),
            arabic,
        });
    }
    out
}

/// Every embeddable installed face, by family then style. Reads every font
/// file, so it takes a moment; call it once and keep the result.
pub fn fonts() -> Vec<FontFace> {
    let mut out = Vec::new();
    for d in dirs() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            out.extend(read_file(&e.path()));
        }
    }
    out.sort_by(|a, b| {
        a.family.to_lowercase().cmp(&b.family.to_lowercase()).then((a.bold, a.italic).cmp(&(b.bold, b.italic)))
    });
    out.dedup_by(|a, b| a.family == b.family && a.bold == b.bold && a.italic == b.italic);
    out
}

/// The face of `family` closest to bold / italic as asked: the exact style,
/// else the same weight, else the regular face.
pub fn pick<'a>(faces: &'a [FontFace], family: &str, bold: bool, italic: bool) -> Option<&'a FontFace> {
    let fam: Vec<&FontFace> = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).collect();
    fam.iter()
        .find(|f| f.bold == bold && f.italic == italic)
        .or_else(|| fam.iter().find(|f| f.bold == bold && !f.italic))
        .or_else(|| fam.iter().find(|f| !f.bold && !f.italic))
        .or(fam.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_fonts_are_found_with_their_styles() {
        let all = fonts();
        if !cfg!(windows) || all.is_empty() {
            return;
        }
        // Arial is on every Windows: four styles, with Arabic.
        let arial = pick(&all, "Arial", true, false).expect("Arial is installed");
        assert!(arial.bold && !arial.italic && arial.arabic, "{arial:?}");
        assert!(pick(&all, "arial", false, false).is_some_and(|f| !f.bold));
        assert!(pick(&all, "No Such Font", false, false).is_none());
    }
}
