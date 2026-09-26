//! Content-stream text extraction: a small state machine over the raw
//! stream bytes. Educational scope — covers BT/ET blocks, Tj/TJ/'/" show
//! operators, Tf font selection, Td/TD/T* line breaks and TJ kerning
//! numbers (large negative gaps become spaces).

use crate::pdf::{Document, Obj};
use std::collections::HashMap;

/// Decode a content-stream byte string (educational WinAnsi-ish mapping:
/// ASCII passes through, high bytes use a small WinAnsi table).
pub fn decode_string(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        out.push(winch(b));
    }
    out
}

fn winch(b: u8) -> char {
    match b {
        0x20..=0x7e => b as char,
        0x80 => '€',
        0x82 => '‚',
        0x84 => '„',
        0x85 => '…',
        0x86 | 0x87 => '†',
        0x89 => '‰',
        0x8a => 'Š',
        0x8b => '‹',
        0x8c => 'Œ',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x99 => '™',
        0x9a => 'š',
        0x9b => '›',
        0x9c => 'œ',
        0x9f => 'Ÿ',
        0xa0..=0xff => b as char,
        _ => ' ',
    }
}

fn is_delim(c: u8) -> bool {
    matches!(
        c,
        b' ' | b'\n' | b'\r' | b'\t' | b'/' | b'>' | b'<' | b'(' | b')' | b'[' | b']'
    )
}

fn read_literal(data: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut depth = 1usize;
    let mut i = 1usize;
    while i < data.len() {
        let b = data[i];
        i += 1;
        match b {
            b'(' => {
                depth += 1;
                out.push(b'(');
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return (out, i);
                }
                out.push(b')');
            }
            b'\\' => {
                if let Some(&e) = data.get(i) {
                    i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                if let Some(&d) = data.get(i) {
                                    if (b'0'..=b'7').contains(&d) {
                                        v = v * 8 + u32::from(d - b'0');
                                        i += 1;
                                    }
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
            }
            other => out.push(other),
        }
    }
    (out, i)
}

fn read_hex(data: &[u8]) -> (Vec<u8>, usize) {
    let mut hex_digits = String::new();
    let mut i = 1usize;
    while i < data.len() {
        let b = data[i];
        i += 1;
        if b == b'>' {
            break;
        }
        if (b as char).is_ascii_hexdigit() {
            hex_digits.push(b as char);
        }
    }
    if hex_digits.len() % 2 == 1 {
        hex_digits.push('0');
    }
    let out: Vec<u8> = (0..hex_digits.len() / 2)
        .map(|k| u8::from_str_radix(&hex_digits[k * 2..k * 2 + 2], 16).unwrap_or(0))
        .collect();
    (out, i)
}

fn read_number(data: &[u8]) -> (f64, usize) {
    let mut end = 0usize;
    while end < data.len()
        && (data[end].is_ascii_digit()
            || data[end] == b'.'
            || ((data[end] == b'-' || data[end] == b'+') && end == 0))
    {
        end += 1;
    }
    let text: String = data[..end].iter().map(|&b| b as char).collect();
    (text.parse().unwrap_or(0.0), end)
}

fn word_at(data: &[u8], i: usize) -> (&'static str, usize) {
    // returns the two-letter operator starting at i if known
    let b1 = data.get(i).copied().unwrap_or(b' ');
    let b2 = data.get(i + 1).copied().unwrap_or(b' ');
    match (b1, b2) {
        (b'B', b'T') => ("BT", 2),
        (b'E', b'T') => ("ET", 2),
        (b'B', b'D') | (b'B', b'C') | (b'B', b'X') => ("BX", 2), // BDC/BX ignored
        (b'T', b'f') => ("Tf", 2),
        (b'T', b'j') => ("Tj", 2),
        (b'T', b'J') => ("TJ", 2),
        (b'T', b'd') => ("Td", 2),
        (b'T', b'D') => ("TD", 2),
        (b'T', b'*') => ("T*", 2),
        (b'T', b'm') => ("Tm", 2),
        (b'\'', _) => ("'", 1),
        (b'"', _) => ("\"", 2),
        _ => ("", 0),
    }
}

/// Interpreter state over one page's content stream.
struct Interp {
    out: String,
    in_text: bool,
    in_tj_array: bool,
    font: String,
    fonts: HashMap<String, String>,
    pending_font: Option<String>,
    pending_number: Option<f64>,
}

impl Interp {
    fn new(fonts: HashMap<String, String>) -> Interp {
        Interp {
            out: String::new(),
            in_text: false,
            in_tj_array: false,
            font: String::new(),
            fonts,
            pending_font: None,
            pending_number: None,
        }
    }

    fn newline(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn show(&mut self, bytes: &[u8]) {
        if !self.in_text {
            return;
        }
        self.out.push_str(&decode_string(bytes));
    }

    fn kern(&mut self, value: f64) {
        // large negative TJ numbers are word gaps
        if self.in_tj_array && value <= -100.0 {
            self.out.push(' ');
        }
    }

    fn run(&mut self, content: &[u8]) {
        let mut i = 0usize;
        while i < content.len() {
            let b = content[i];
            match b {
                b'(' => {
                    let (s, next) = read_literal(&content[i..]);
                    i += next;
                    self.show(&s);
                }
                b'<' if content.get(i + 1) != Some(&b'<') => {
                    let (s, next) = read_hex(&content[i..]);
                    i += next;
                    self.show(&s);
                }
                b'/' => {
                    let mut j = i + 1;
                    let mut name = String::new();
                    while j < content.len() && !is_delim(content[j]) {
                        name.push(content[j] as char);
                        j += 1;
                    }
                    i = j;
                    self.pending_font = Some(name);
                }
                b'-' | b'+' | b'.' | b'0'..=b'9' => {
                    let (v, next) = read_number(&content[i..]);
                    i += next;
                    if self.in_tj_array {
                        self.kern(v);
                    }
                    self.pending_number = Some(v);
                }
                b'[' => {
                    if self.in_text {
                        self.in_tj_array = true;
                    }
                    i += 1;
                }
                b']' => {
                    self.in_tj_array = false;
                    i += 1;
                }
                _ => {
                    let (word, len) = word_at(content, i);
                    match word {
                        "BT" => {
                            self.in_text = true;
                            i += len;
                        }
                        "ET" => {
                            self.in_text = false;
                            i += len;
                        }
                        "Tf" => {
                            if let Some(name) = self.pending_font.take() {
                                self.font = self.fonts.get(&name).cloned().unwrap_or(name);
                            }
                            i += len;
                        }
                        "Td" | "TD" => {
                            // operands were numbers; a line move in BT context
                            if self.in_text {
                                self.newline();
                            }
                            i += len;
                        }
                        "T*" => {
                            if self.in_text {
                                self.newline();
                            }
                            i += len;
                        }
                        "Tm" => {
                            i += len;
                        }
                        "'" | "\"" => {
                            if self.in_text {
                                self.newline();
                            }
                            i += len;
                        }
                        _ => i += 1,
                    }
                }
            }
        }
    }
}

/// Extract text from one page's concatenated content stream.
pub fn extract_from_stream(content: &[u8], fonts: &HashMap<String, String>) -> String {
    let mut it = Interp::new(fonts.clone());
    it.run(content);
    it.out
}

/// Extract text for every page in order.
pub fn extract_pages(doc: &Document) -> Vec<String> {
    doc.page_numbers()
        .iter()
        .map(|&num| extract_page(doc, num).unwrap_or_default())
        .collect()
}

pub fn extract_page(doc: &Document, num: i64) -> Option<String> {
    let page = doc.objects.get(&num)?.as_dict()?.clone();
    let fonts = collect_fonts(doc, &page);
    let content = collect_content(doc, &page)?;
    Some(extract_from_stream(&content, &fonts))
}

fn collect_fonts(doc: &Document, page: &HashMap<String, Obj>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut resources = page.get("Resources").cloned();
    if resources.is_none() {
        // pages may inherit resources from the parent
        if let Some(Obj::Ref(parent, _)) = page.get("Parent") {
            if let Some(parent_dict) = doc.objects.get(parent).and_then(|o| o.as_dict()) {
                resources = parent_dict.get("Resources").cloned();
            }
        }
    }
    if let Some(res) = resources.and_then(|r| r.as_dict().cloned()) {
        if let Some(Obj::Dict(fonts)) = res.get("Font") {
            for (name, fobj) in fonts {
                // the font dict may be an indirect reference
                let fdict = fobj.as_dict().cloned().or_else(|| {
                    fobj.as_ref().and_then(|(num, _)| {
                        doc.objects.get(&num).and_then(|o| o.as_dict()).cloned()
                    })
                });
                if let Some(d) = fdict {
                    let base = d
                        .get("BaseFont")
                        .and_then(|b| b.as_name())
                        .unwrap_or("Unknown")
                        .to_string();
                    out.insert(name.clone(), base);
                }
            }
        }
    }
    out
}

fn collect_content(doc: &Document, page: &HashMap<String, Obj>) -> Option<Vec<u8>> {
    let contents = page.get("Contents")?;
    let mut refs: Vec<i64> = Vec::new();
    match contents {
        Obj::Ref(n, _) => refs.push(*n),
        Obj::Array(items) => {
            for item in items {
                if let Obj::Ref(n, _) = item {
                    refs.push(*n);
                }
            }
        }
        _ => return None,
    }
    let mut out = Vec::new();
    for n in &refs {
        if let Some(Obj::Stream(_dict, raw)) = doc.objects.get(n) {
            out.extend_from_slice(raw);
            out.push(b'\n');
        }
    }
    // FlateDecode when the page's content dicts ask for it
    decode_streams(doc, &refs, &mut out);
    Some(out)
}

fn decode_streams(doc: &Document, refs: &[i64], out: &mut Vec<u8>) {
    // if any content stream is FlateDecode, rebuild `out` from decoded parts
    let mut decoded = Vec::new();
    for n in refs {
        let Some(Obj::Stream(dict, raw)) = doc.objects.get(n) else {
            continue;
        };
        let filter = dict
            .get("Filter")
            .and_then(|f| match f {
                Obj::Name(n) => Some(vec![n.clone()]),
                Obj::Array(a) => Some(
                    a.iter()
                        .filter_map(|o| o.as_name().map(|s| s.to_string()))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        if filter.iter().any(|f| f == "FlateDecode") {
            match crate::inflate::zlib_decompress(raw) {
                Ok(part) => decoded.extend_from_slice(&part),
                Err(_) => return, // corrupt stream: keep raw fallback
            }
        } else {
            decoded.extend_from_slice(raw);
        }
        decoded.push(b'\n');
    }
    *out = decoded;
}

pub fn extract_page_text(doc: &Document, num: i64) -> String {
    extract_page(doc, num).unwrap_or_default()
}

/// Test bridge: collect font names for a page dict.
pub fn collect_fonts_public(doc: &Document, page: &HashMap<String, Obj>) -> HashMap<String, String> {
    collect_fonts(doc, page)
}
