//! PDF objects: parsing from bytes and writing back.

use std::fmt::Write as _;

/// An object number and generation.
pub type Ref = (u32, u16);

#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<Obj>),
    Dict(Dict),
    /// A stream: its dictionary and its data as stored (still encoded).
    Stream(Dict, Vec<u8>),
    Ref(Ref),
}

/// A dictionary, in file order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dict(pub Vec<(Vec<u8>, Obj)>);

impl Dict {
    pub fn get(&self, key: &[u8]) -> Option<&Obj> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn set(&mut self, key: &[u8], v: Obj) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = v,
            None => self.0.push((key.to_vec(), v)),
        }
    }

    pub fn remove(&mut self, key: &[u8]) {
        self.0.retain(|(k, _)| k != key);
    }

    pub fn name(&self, key: &[u8]) -> Option<&[u8]> {
        match self.get(key) {
            Some(Obj::Name(n)) => Some(n),
            _ => None,
        }
    }
}

impl Obj {
    pub fn name(n: &str) -> Obj {
        Obj::Name(n.as_bytes().to_vec())
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Obj::Int(i) => Some(*i),
            Obj::Real(r) if r.fract() == 0.0 => Some(*r as i64),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Obj::Int(i) => Some(*i as f32),
            Obj::Real(r) => Some(*r as f32),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) | Obj::Stream(d, _) => Some(d),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_ref(&self) -> Option<Ref> {
        match self {
            Obj::Ref(r) => Some(*r),
            _ => None,
        }
    }

    /// Numbers of an array of numbers.
    pub fn numbers(&self) -> Option<Vec<f32>> {
        self.as_array()?.iter().map(Obj::as_f32).collect()
    }

    /// A PDF text string as Rust text (UTF-16 with a byte order mark, UTF-8
    /// with one, or PDFDocEncoding, treated as Latin-1).
    pub fn text(&self) -> Option<String> {
        let Obj::Str(b) = self else { return None };
        Some(decode_text(b))
    }
}

pub fn decode_text(b: &[u8]) -> String {
    if b.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&b[3..]).into_owned()
    } else {
        b.iter().map(|&c| c as char).collect()
    }
}

/// A PDF text string for `s`: plain bytes for ASCII, UTF-16BE otherwise.
pub fn encode_text(s: &str) -> Obj {
    if s.is_ascii() {
        Obj::Str(s.as_bytes().to_vec())
    } else {
        let mut b = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            b.extend_from_slice(&u.to_be_bytes());
        }
        Obj::Str(b)
    }
}

// ---------------------------------------------------------------- parsing

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

type PResult<T> = Result<T, ParseError>;

fn err<T>(msg: &str, pos: usize) -> PResult<T> {
    Err(ParseError(format!("{msg} at byte {pos}")))
}

pub fn is_white(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// Skips white space and comments.
pub fn skip_ws(b: &[u8], p: &mut usize) {
    while *p < b.len() {
        if is_white(b[*p]) {
            *p += 1;
        } else if b[*p] == b'%' {
            while *p < b.len() && b[*p] != b'\n' && b[*p] != b'\r' {
                *p += 1;
            }
        } else {
            break;
        }
    }
}

/// A run of regular characters (a keyword or number).
pub fn token<'a>(b: &'a [u8], p: &mut usize) -> &'a [u8] {
    let s = *p;
    while *p < b.len() && !is_white(b[*p]) && !is_delim(b[*p]) {
        *p += 1;
    }
    &b[s..*p]
}

/// Deepest nesting of arrays and dictionaries accepted (hostile files).
const MAX_DEPTH: usize = 64;

/// Parses one object at `p`. Stream data is not read: a dictionary followed
/// by `stream` comes back as a plain dictionary, with `p` after the dictionary.
pub fn parse(b: &[u8], p: &mut usize) -> PResult<Obj> {
    parse_depth(b, p, 0)
}

fn parse_depth(b: &[u8], p: &mut usize, depth: usize) -> PResult<Obj> {
    if depth > MAX_DEPTH {
        return err("nested too deeply", *p);
    }
    skip_ws(b, p);
    let Some(&c) = b.get(*p) else { return err("unexpected end", *p) };
    match c {
        b'/' => {
            *p += 1;
            Ok(Obj::Name(name(b, p)))
        }
        b'(' => literal(b, p).map(Obj::Str),
        b'<' if b.get(*p + 1) == Some(&b'<') => {
            *p += 2;
            let mut d = Dict::default();
            loop {
                skip_ws(b, p);
                match b.get(*p) {
                    Some(b'>') if b.get(*p + 1) == Some(&b'>') => {
                        *p += 2;
                        return Ok(Obj::Dict(d));
                    }
                    Some(b'/') => {
                        *p += 1;
                        let k = name(b, p);
                        let v = parse_depth(b, p, depth + 1)?;
                        // A repeated key: the first wins, as most readers do.
                        if d.get(&k).is_none() {
                            d.0.push((k, v));
                        }
                    }
                    None => return err("unterminated dictionary", *p),
                    _ => {
                        // Junk inside a dictionary: skip a token rather than fail.
                        let t = token(b, p);
                        if t.is_empty() {
                            *p += 1;
                        }
                    }
                }
            }
        }
        b'<' => hex(b, p).map(Obj::Str),
        b'[' => {
            *p += 1;
            let mut a = Vec::new();
            loop {
                skip_ws(b, p);
                match b.get(*p) {
                    Some(b']') => {
                        *p += 1;
                        return Ok(Obj::Array(a));
                    }
                    None => return err("unterminated array", *p),
                    _ => a.push(parse_depth(b, p, depth + 1)?),
                }
            }
        }
        _ => {
            let start = *p;
            let t = token(b, p);
            match t {
                b"true" => Ok(Obj::Bool(true)),
                b"false" => Ok(Obj::Bool(false)),
                b"null" => Ok(Obj::Null),
                _ if t.is_empty() => {
                    *p += 1;
                    err("unexpected character", start)
                }
                _ => {
                    let n = number(t).ok_or_else(|| ParseError(format!("bad token at byte {start}")))?;
                    // An integer may start a reference: "12 0 R".
                    if let Obj::Int(num) = n
                        && num >= 0
                    {
                        let mut q = *p;
                        skip_ws(b, &mut q);
                        let gen_t = token(b, &mut q);
                        if let Some(Obj::Int(g)) = number(gen_t)
                            && (0..=65535).contains(&g)
                        {
                            skip_ws(b, &mut q);
                            if b.get(q) == Some(&b'R') && b.get(q + 1).is_none_or(|c| is_white(*c) || is_delim(*c)) {
                                *p = q + 1;
                                return Ok(Obj::Ref((num as u32, g as u16)));
                            }
                        }
                    }
                    Ok(n)
                }
            }
        }
    }
}

fn number(t: &[u8]) -> Option<Obj> {
    let s = std::str::from_utf8(t).ok()?;
    if s.is_empty() {
        return None;
    }
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        return s.parse::<i64>().ok().map(Obj::Int);
    }
    // PDF reals have no exponent; be lenient with "--1" style junk by failing.
    s.parse::<f64>().ok().filter(|v| v.is_finite()).map(Obj::Real)
}

fn name(b: &[u8], p: &mut usize) -> Vec<u8> {
    let mut out = Vec::new();
    while *p < b.len() && !is_white(b[*p]) && !is_delim(b[*p]) {
        if b[*p] == b'#'
            && let (Some(h), Some(l)) = (b.get(*p + 1).and_then(hexval), b.get(*p + 2).and_then(hexval))
        {
            out.push(h << 4 | l);
            *p += 3;
            continue;
        }
        out.push(b[*p]);
        *p += 1;
    }
    out
}

fn hexval(c: &u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn literal(b: &[u8], p: &mut usize) -> PResult<Vec<u8>> {
    let start = *p;
    *p += 1;
    let mut depth = 1;
    let mut out = Vec::new();
    while *p < b.len() {
        let c = b[*p];
        *p += 1;
        match c {
            b'(' => {
                depth += 1;
                out.push(c);
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(out);
                }
                out.push(c);
            }
            b'\\' => {
                let Some(&e) = b.get(*p) else { break };
                *p += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'\r' => {
                        if b.get(*p) == Some(&b'\n') {
                            *p += 1;
                        }
                    }
                    b'\n' => {}
                    b'0'..=b'7' => {
                        let mut v = (e - b'0') as u32;
                        for _ in 0..2 {
                            match b.get(*p) {
                                Some(d @ b'0'..=b'7') => {
                                    v = v * 8 + (d - b'0') as u32;
                                    *p += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(v as u8);
                    }
                    other => out.push(other),
                }
            }
            _ => out.push(c),
        }
    }
    err("unterminated string", start)
}

fn hex(b: &[u8], p: &mut usize) -> PResult<Vec<u8>> {
    let start = *p;
    *p += 1;
    let mut out = Vec::new();
    let mut hi: Option<u8> = None;
    while *p < b.len() {
        let c = b[*p];
        *p += 1;
        if c == b'>' {
            if let Some(h) = hi {
                out.push(h << 4);
            }
            return Ok(out);
        }
        if let Some(v) = hexval(&c) {
            match hi.take() {
                Some(h) => out.push(h << 4 | v),
                None => hi = Some(v),
            }
        }
    }
    err("unterminated hex string", start)
}

// ---------------------------------------------------------------- writing

/// Appends `o` in PDF syntax.
pub fn write(o: &Obj, out: &mut Vec<u8>) {
    match o {
        Obj::Null => out.extend_from_slice(b"null"),
        Obj::Bool(v) => out.extend_from_slice(if *v { b"true" } else { b"false" }),
        Obj::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Obj::Real(r) => out.extend_from_slice(real(*r).as_bytes()),
        Obj::Name(n) => write_name(n, out),
        Obj::Str(s) => {
            out.push(b'(');
            for &c in s {
                match c {
                    b'(' | b')' | b'\\' => {
                        out.push(b'\\');
                        out.push(c);
                    }
                    b'\r' => out.extend_from_slice(b"\\r"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    _ => out.push(c),
                }
            }
            out.push(b')');
        }
        Obj::Array(a) => {
            out.push(b'[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write(v, out);
            }
            out.push(b']');
        }
        Obj::Dict(d) => write_dict(d, out),
        Obj::Stream(d, data) => {
            let mut d = d.clone();
            d.set(b"Length", Obj::Int(data.len() as i64));
            write_dict(&d, out);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(data);
            out.extend_from_slice(b"\nendstream");
        }
        Obj::Ref((n, g)) => out.extend_from_slice(format!("{n} {g} R").as_bytes()),
    }
}

fn write_dict(d: &Dict, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    for (k, v) in &d.0 {
        write_name(k, out);
        out.push(b' ');
        write(v, out);
    }
    out.extend_from_slice(b">>");
}

fn write_name(n: &[u8], out: &mut Vec<u8>) {
    out.push(b'/');
    for &c in n {
        if c <= b' ' || c >= 0x7F || is_delim(c) || c == b'#' {
            let _ = write!(Hex(out), "#{c:02X}");
        } else {
            out.push(c);
        }
    }
}

struct Hex<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Hex<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// A real number without an exponent, trimmed.
pub fn real(v: f64) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Obj {
        parse(s.as_bytes(), &mut 0).unwrap()
    }

    #[test]
    fn parses_the_basics() {
        assert_eq!(p("  42 "), Obj::Int(42));
        assert_eq!(p("-3.5"), Obj::Real(-3.5));
        assert_eq!(p(".5"), Obj::Real(0.5));
        assert_eq!(p("12 0 R"), Obj::Ref((12, 0)));
        assert_eq!(p("[1 2 R 3]"), Obj::Array(vec![Obj::Ref((1, 2)), Obj::Int(3)]));
        assert_eq!(p("/A#20B"), Obj::Name(b"A B".to_vec()));
        assert_eq!(p("(a\\(b\\)c\\101(d))"), Obj::Str(b"a(b)cA(d)".to_vec()));
        assert_eq!(p("<48 65 6C6C 6F7>"), Obj::Str(b"Hello\x70".to_vec()));
        let d = p("<< /Type /Annot /Rect [0 0 1.5 2] /F 4 /P 3 0 R % comment\n >>");
        let d = d.as_dict().unwrap();
        assert_eq!(d.name(b"Type"), Some(&b"Annot"[..]));
        assert_eq!(d.get(b"Rect").unwrap().numbers().unwrap(), vec![0.0, 0.0, 1.5, 2.0]);
        assert_eq!(d.get(b"P"), Some(&Obj::Ref((3, 0))));
    }

    #[test]
    fn hostile_input_fails_cleanly() {
        let deep = "[".repeat(1000);
        assert!(parse(deep.as_bytes(), &mut 0).is_err());
        assert!(parse(b"(unterminated", &mut 0).is_err());
        assert!(parse(b"<< /A [1 2", &mut 0).is_err());
        assert!(parse(b"", &mut 0).is_err());
    }

    #[test]
    fn writes_what_it_reads() {
        let src = "<</Type /Annot /Contents (a\\(b\\)\\\\) /N /A#20B /V [1 0.25 -2] /R 4 0 R /T true /Z null>>";
        let o = p(src);
        let mut out = Vec::new();
        write(&o, &mut out);
        assert_eq!(p(std::str::from_utf8(&out).unwrap()), o);
    }

    #[test]
    fn text_strings() {
        assert_eq!(encode_text("abc"), Obj::Str(b"abc".to_vec()));
        let ar = encode_text("مرحبا");
        assert_eq!(ar.text().unwrap(), "مرحبا");
        assert_eq!(real(1.0), "1");
        assert_eq!(real(0.12345), "0.1235");
    }
}
