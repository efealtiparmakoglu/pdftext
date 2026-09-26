//! pdftext — educational PDF text extractor.

pub mod inflate;
pub mod lexer;
pub mod pdf;
pub mod text;

pub use pdf::{Document, Obj};
