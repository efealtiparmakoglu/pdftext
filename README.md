# pdftext

**Educational PDF text extractor in pure Rust — object scanner, FlateDecode, content-stream text operators. Zero dependencies.**

[![CI](https://github.com/efealtiparmakoglu/pdftext/actions/workflows/ci.yml/badge.svg)](https://github.com/efealtiparmakoglu/pdftext/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.82%2B-orange.svg)](https://www.rust-lang.org)

```
$ pdftext --pages 1 report.pdf
--- page 1 ---
Hello PDF world
Escapes: (paren) \ backslash AB
```

Written to understand PDFs from the inside out: a lexical scanner for the
object language, a DEFLATE decompressor, the page tree and a content
stream interpreter for the text-showing operators.

## Why pdftext

- **DEFLATE from scratch.** Stored/fixed/dynamic Huffman blocks with a
  puff-style canonical decoder; zlib wrapper verifies the adler32 at the
  position where inflate stopped — tolerating trailing bytes before
  `endstream`.
- **Real object parsing.** Names with `#xx` escapes, literal strings
  with balanced parens and octal escapes, hex strings, nested
  arrays/dictionaries, indirect references and stream bodies
  (`/Length`-based with an `endstream` scan fallback).
- **Content-stream interpreter.** `BT/ET` blocks, `Tj`/`TJ`/`'`/`"` show
  operators, `Tf` font selection, `Td/TD/T*` line breaks; large negative
  `TJ` numbers become word gaps.
- **Page tree aware.** Walks `/Catalog → /Pages → /Kids` in document
  order; `--pages 1-3` selects pages.
- **Zero dependencies.**

## Known limitations

This is an educational extractor, not a pdf2text replacement:
- Only simple encodings (ASCII/WinAnsi-ish byte mapping); fonts with
  embedded CMaps or CID encodings produce raw byte output.
- Cross-reference streams, object streams (PDF 1.5+) and encrypted
  files are not supported — use the linear-scan fallback instead.
- Text layout is line-oriented; columns/tables appear in reading order
  only.

## Usage

```sh
pdftext report.pdf
pdftext --pages 1-3 report.pdf
```

Exit codes: 0 ok, 1 read/parse failure, 2 usage error.

## Development

```sh
cargo test
cargo clippy --all-targets
```

Tests cover the lexer (escapes, hex strings, comments, numbers), DEFLATE
(stored/fixed/dynamic real-world streams, corruption), zlib adler
placement, object scanning, page-tree walking, font resolution through
indirect references and text extraction with escapes and TJ kerning.
