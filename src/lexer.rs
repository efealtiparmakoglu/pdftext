//! PDF object lexer — tokens for the PDF spec's lexical vocabulary.
//!
//! Handles names (`/Name` with `#xx` hex escapes), numbers (int/real),
//! literal strings (nested parens, `\n` `\(` `\\` `\ddd` octal escapes),
//! hex strings (`<A0B1>`), dictionaries (`<<` / `>>`), arrays, booleans,
//! null and keywords.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Name(String),
    Str(Vec<u8>),
    Int(i64),
    Real(f64),
    Bool(bool),
    Null,
    ArrayStart,
    ArrayEnd,
    DictStart,
    DictEnd,
    Keyword(String),
}

pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Lexer { data, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.data.len() {
            match self.data[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x00 => self.pos += 1,
                b'%' => {
                    // comment: skip to end of line
                    while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    /// Read the next token; returns None at end of input.
    pub fn next_token(&mut self) -> Option<Result<Token, String>> {
        self.skip_ws();
        let b = *self.data.get(self.pos)?;
        Some(match b {
            b'/' => {
                self.pos += 1;
                let mut name = String::new();
                while self.pos < self.data.len() {
                    let c = self.data[self.pos];
                    if c == b'#' && self.pos + 2 < self.data.len() {
                        let hi = (self.data[self.pos + 1] as char).to_digit(16)?;
                        let lo = (self.data[self.pos + 2] as char).to_digit(16)?;
                        name.push(((hi * 16 + lo) as u8) as char);
                        self.pos += 3;
                        continue;
                    }
                    if is_delim_or_ws(c) {
                        break;
                    }
                    name.push(c as char);
                    self.pos += 1;
                }
                Ok(Token::Name(name))
            }
            b'(' => {
                self.pos += 1;
                match self.literal_string() {
                    Ok(s) => Ok(Token::Str(s)),
                    Err(e) => Err(e),
                }
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Ok(Token::DictStart)
            }
            b'<' => {
                self.pos += 1;
                let mut hex_digits = String::new();
                while self.pos < self.data.len() {
                    let c = self.data[self.pos];
                    if c == b'>' {
                        self.pos += 1;
                        break;
                    }
                    if (c as char).is_ascii_hexdigit() {
                        hex_digits.push(c as char);
                    }
                    self.pos += 1;
                }
                if hex_digits.len() % 2 == 1 {
                    hex_digits.push('0');
                }
                let bytes: Vec<u8> = (0..hex_digits.len() / 2)
                    .map(|i| u8::from_str_radix(&hex_digits[i * 2..i * 2 + 2], 16).unwrap_or(0))
                    .collect();
                Ok(Token::Str(bytes))
            }
            b'[' => {
                self.pos += 1;
                Ok(Token::ArrayStart)
            }
            b']' => {
                self.pos += 1;
                Ok(Token::ArrayEnd)
            }
            b'>' if self.data.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Ok(Token::DictEnd)
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = self.pos;
                let mut is_real = false;
                while self.pos < self.data.len() {
                    let c = self.data[self.pos];
                    if c.is_ascii_digit() {
                        self.pos += 1;
                    } else if c == b'.' && !is_real {
                        is_real = true;
                        self.pos += 1;
                    } else if (c == b'+' || c == b'-') && self.pos == start {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let text = std::str::from_utf8(&self.data[start..self.pos]).unwrap_or("0");
                if is_real {
                    Ok(Token::Real(text.parse().unwrap_or(0.0)))
                } else {
                    match text.parse::<i64>() {
                        Ok(v) => Ok(Token::Int(v)),
                        Err(_) => Ok(Token::Real(text.parse().unwrap_or(0.0))),
                    }
                }
            }
            _ => {
                let start = self.pos;
                while self.pos < self.data.len() && !is_delim_or_ws(self.data[self.pos]) {
                    self.pos += 1;
                }
                if start == self.pos {
                    self.pos += 1;
                    return Some(Err(format!("stray byte at {}", start)));
                }
                let word = std::str::from_utf8(&self.data[start..self.pos]).unwrap_or("");
                Ok(match word {
                    "true" => Token::Bool(true),
                    "false" => Token::Bool(false),
                    "null" => Token::Null,
                    k => Token::Keyword(k.to_string()),
                })
            }
        })
    }

    /// Literal string with balanced parens and escape sequences.
    fn literal_string(&mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        loop {
            let b = *self
                .data
                .get(self.pos)
                .ok_or_else(|| "unterminated string".to_string())?;
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b'(');
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    out.push(b')');
                }
                b'\\' => {
                    let e = *self
                        .data
                        .get(self.pos)
                        .ok_or_else(|| "unterminated escape".to_string())?;
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'(' => out.push(b'('),
                        b')' => out.push(b')'),
                        b'\\' => out.push(b'\\'),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                if let Some(&d) = self.data.get(self.pos) {
                                    if (b'0'..=b'7').contains(&d) {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        b'\n' => {} // line continuation
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
    }

    /// Skip to the next occurrence of `word` (keyword search).
    pub fn skip_until(&mut self, word: &[u8]) -> bool {
        if let Some(found) = find_sub(self.data, word, self.pos) {
            self.pos = found + word.len();
            return true;
        }
        false
    }
}

pub fn find_sub(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn is_delim_or_ws(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' | b' ' | b'\t'
            | b'\r' | b'\n' | 0x0c | 0x00
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(data: &[u8]) -> Vec<Token> {
        let mut lx = Lexer::new(data);
        let mut out = Vec::new();
        while let Some(r) = lx.next_token() {
            out.push(r.unwrap());
        }
        out
    }

    #[test]
    fn tokens_dictionary_and_array() {
        let toks = tokens(b"<< /Name /Val [1 2.5 (str\\(x\\))] >> true null KW");
        assert_eq!(
            toks,
            vec![
                Token::DictStart,
                Token::Name("Name".into()),
                Token::Name("Val".into()),
                Token::ArrayStart,
                Token::Int(1),
                Token::Real(2.5),
                Token::Str(b"str(x)".to_vec()),
                Token::ArrayEnd,
                Token::DictEnd,
                Token::Bool(true),
                Token::Null,
                Token::Keyword("KW".into()),
            ]
        );
    }

    #[test]
    fn octal_escapes() {
        assert_eq!(tokens(b"(\\101\\102)"), vec![Token::Str(b"AB".to_vec())]);
    }

    #[test]
    fn hex_string_and_hash_escape() {
        assert_eq!(tokens(b"<4869>"), vec![Token::Str(b"Hi".to_vec())]);
        // /A#20B decodes the #20 as a space
        assert_eq!(tokens(b"/A#20B"), vec![Token::Name("A B".into())]);
    }

    #[test]
    fn negative_numbers_and_comments() {
        assert_eq!(
            tokens(b"% a comment\n-72.5"),
            vec![Token::Real(-72.5)]
        );
    }
}
