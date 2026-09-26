//! PDF document model: scanned objects, page tree, resolved page content.

use crate::lexer::{find_sub, Lexer, Token};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Name(String),
    Str(Vec<u8>),
    Int(i64),
    Real(f64),
    Bool(bool),
    Null,
    Array(Vec<Obj>),
    Dict(HashMap<String, Obj>),
    Ref(i64, i64),
    Keyword(String),
    Stream(HashMap<String, Obj>, Vec<u8>),
}

impl Obj {
    pub fn as_dict(&self) -> Option<&HashMap<String, Obj>> {
        if let Obj::Dict(d) = self {
            Some(d)
        } else {
            None
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        if let Obj::Int(i) = self {
            Some(*i)
        } else {
            None
        }
    }
    pub fn as_name(&self) -> Option<&str> {
        if let Obj::Name(n) = self {
            Some(n)
        } else {
            None
        }
    }
    pub fn as_ref(&self) -> Option<(i64, i64)> {
        if let Obj::Ref(n, g) = self {
            Some((*n, *g))
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Obj>> {
        if let Obj::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
}

/// Every indirect object found by a linear scan: key is the object number.
pub struct Document {
    pub objects: HashMap<i64, Obj>,
    pub root_ref: Option<(i64, i64)>,
}

impl Document {
    pub fn get(&self, num: i64) -> Option<&Obj> {
        self.objects.get(&num)
    }
    /// Resolve one reference hop (not recursive).
    pub fn resolve(&self, o: &Obj) -> Option<&Obj> {
        if let Obj::Ref(n, _) = o {
            self.objects.get(n)
        } else {
            None
        }
    }

    /// Page object numbers in document order, walking the /Pages tree.
    pub fn page_numbers(&self) -> Vec<i64> {
        let mut out = Vec::new();
        let root = self.root_ref.and_then(|(n, _)| self.objects.get(&n)).cloned();
        let pages_ref = root.as_ref().and_then(|cat| {
            cat.as_dict()
                .and_then(|d| d.get("Pages"))
                .and_then(|p| p.as_ref())
        });
        if let Some((num, _)) = pages_ref {
            self.walk_pages(num, &mut out, 0);
        }
        if out.is_empty() {
            // fallback: any object that looks like a page
            let mut nums: Vec<i64> = self
                .objects
                .iter()
                .filter(|(_, o)| {
                    o.as_dict()
                        .and_then(|d| d.get("Type"))
                        .and_then(|t| t.as_name())
                        == Some("Page")
                })
                .map(|(k, _)| *k)
                .collect();
            nums.sort();
            out = nums;
        }
        out
    }

    fn walk_pages(&self, num: i64, out: &mut Vec<i64>, depth: usize) {
        if depth > 32 {
            return;
        }
        let Some(obj) = self.objects.get(&num) else {
            return;
        };
        let Some(dict) = obj.as_dict() else {
            return;
        };
        let ty = dict.get("Type").and_then(|t| t.as_name()).unwrap_or("");
        if ty == "Pages" {
            if let Some(kids) = dict.get("Kids").and_then(|k| k.as_array()) {
                for kid in kids {
                    if let Some((n, _)) = kid.as_ref() {
                        self.walk_pages(n, out, depth + 1);
                    }
                }
            }
        } else if ty == "Page" {
            out.push(num);
        }
    }
}

/// Scan the whole file for `N G obj ... endobj` and parse each object.
pub fn scan_document(data: &[u8]) -> Result<Document, String> {
    let mut objects: HashMap<i64, Obj> = HashMap::new();
    let mut pos = 0usize;
    while let Some(i) = find_sub(data, b" obj", pos) {
        // object header: "N G obj" — walk back over "N G "
        let head_start = find_object_start(data, i)?;
        let mut lx = Lexer::new(&data[head_start..i]);
        let num = match lx.next_token() {
            Some(Ok(Token::Int(n))) => n,
            _ => {
                pos = i + 4;
                continue;
            }
        };
        match lx.next_token() {
            Some(Ok(Token::Int(_))) => {}
            _ => {
                pos = i + 4;
                continue;
            }
        }
        let mut lx2 = Lexer::new(&data[i + 4..]);
        let body_start = i + 4;
        let obj = parse_object(&mut lx2)?;
        let mut obj = obj;
        // stream bodies
        while lx2.pos < lx2.data.len() && lx2.data[lx2.pos].is_ascii_whitespace() {
            lx2.pos += 1;
        }
        if lx2.data[lx2.pos..].starts_with(b"stream") {
            lx2.pos += 6;
            if lx2.data.get(lx2.pos) == Some(&b'\r') {
                lx2.pos += 1;
            }
            if lx2.data.get(lx2.pos) == Some(&b'\n') {
                lx2.pos += 1;
            }
            // prefer the declared /Length; fall back to an endstream scan
            let declared_len = obj.as_dict().and_then(|d| d.get("Length")).and_then(|l| l.as_int());
            let declared = declared_len.map(|l| l as usize);
            let end_stream = match declared {
                Some(l) => body_start + lx2.pos + l,
                None => find_sub(data, b"endstream", body_start + lx2.pos)
                    .unwrap_or(data.len()),
            };
            let raw = &data[body_start + lx2.pos..end_stream];
            let dict = match &obj {
                Obj::Dict(d) => d.clone(),
                _ => HashMap::new(),
            };
            obj = Obj::Stream(dict, raw.to_vec());
        }
        objects.insert(num, obj);
        pos = i + 4;
    }

    // find /Root via the trailer dictionary
    let mut root_ref = None;
    if let Some(ti) = find_sub(data, b"trailer", 0) {
        let mut lx = Lexer::new(&data[ti..]);
        lx.skip_until(b">>"); // naive: parse the trailer dict fully
        let mut lx = Lexer::new(&data[ti + 7..]);
        if let Ok(Obj::Dict(trailer)) = parse_object(&mut lx) {
            root_ref = trailer.get("Root").and_then(|r| r.as_ref());
        }
    }
    Ok(Document { objects, root_ref })
}

fn find_object_start(data: &[u8], obj_kw: usize) -> Result<usize, String> {
    // scan back over "N G " (two integers)
    let mut i = obj_kw;
    let mut ints = 0;
    while i > 0 && ints < 2 {
        let mut j = i;
        while j > 0 && data[j - 1].is_ascii_whitespace() {
            j -= 1;
        }
        let mut k = j;
        while k > 0 && data[k - 1].is_ascii_digit() {
            k -= 1;
        }
        if k == j {
            break;
        }
        i = k;
        ints += 1;
    }
    Ok(i)
}

/// Parse one PDF object from the current lexer position.
pub fn parse_object(lx: &mut Lexer) -> Result<Obj, String> {
    let Some(tok) = lx.next_token() else {
        return Err("eof".into());
    };
    let tok = tok?;
    parse_from_token(lx, tok)
}

fn parse_from_token(lx: &mut Lexer, first: Token) -> Result<Obj, String> {
    match first {
        Token::Int(i) => {
            // possible reference: "N G R"
            let save = lx.pos;
            if let Some(Ok(Token::Int(g))) = lx.next_token() {
                if let Some(Ok(Token::Keyword(k))) = lx.next_token() {
                    if k == "R" {
                        return Ok(Obj::Ref(i, g));
                    }
                }
                lx.pos = save;
            } else {
                lx.pos = save;
            }
            Ok(Obj::Int(i))
        }
        Token::ArrayStart => {
            let mut items = Vec::new();
            loop {
                match lx.next_token() {
                    Some(Ok(Token::ArrayEnd)) => break,
                    Some(Ok(t)) => items.push(parse_from_token(lx, t)?),
                    Some(Err(e)) => return Err(e),
                    None => return Err("unterminated array".into()),
                }
            }
            Ok(Obj::Array(items))
        }
        Token::DictStart => {
            let mut dict = HashMap::new();
            loop {
                match lx.next_token() {
                    Some(Ok(Token::DictEnd)) => break,
                    Some(Ok(Token::Name(key))) => {
                        dict.insert(key, parse_object(lx)?);
                    }
                    Some(Err(e)) => return Err(e),
                    None => return Err("unterminated dict".into()),
                    _ => {}
                }
            }
            Ok(Obj::Dict(dict))
        }
        Token::Name(n) => Ok(Obj::Name(n)),
        Token::Str(s) => Ok(Obj::Str(s)),
        Token::Real(r) => Ok(Obj::Real(r)),
        Token::Bool(b) => Ok(Obj::Bool(b)),
        Token::Null => Ok(Obj::Null),
        Token::Keyword(k) => Ok(Obj::Keyword(k)),
        Token::DictEnd | Token::ArrayEnd => Err("unexpected closing token".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_PDF: &[u8] = b"%PDF-1.4\n\
1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Kids [4 0 R] /Count 1 >>\nendobj\n\
3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n\
4 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>\nendobj\n\
5 0 obj\n<< /Length 44 >>\nstream\nBT /F1 12 Tf 72 720 Td (Hello PDF) Tj ET\nendstream\nendobj\n\
trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n0\n%%EOF\n";

    #[test]
    fn scan_finds_all_objects() {
        let doc = scan_document(SIMPLE_PDF).unwrap();
        assert_eq!(doc.page_numbers(), vec![4]);
        assert_eq!(doc.root_ref, Some((1, 0)));
    }

    #[test]
    fn font_basefont_resolved() {
        let doc = scan_document(SIMPLE_PDF).unwrap();
        let page = doc.get(4).unwrap().as_dict().unwrap();
        let fonts = super::super::text::collect_fonts_public(&doc, page);
        assert_eq!(fonts.get("F1").map(|s| s.as_str()), Some("Helvetica"));
    }
}
