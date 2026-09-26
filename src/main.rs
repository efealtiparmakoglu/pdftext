// pdftext CLI — extract text from PDF content streams.

use pdftext::text;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const HELP: &str = "\
pdftext — extract text from PDF content streams (educational parser)

USAGE:
  pdftext [--pages 1-3] FILE.pdf

OPTIONS:
  --pages RANGE   page selection, e.g. 3 or 1-3 (default: all)
  -h, --help      show this help
  -V, --version   show version";

fn main() -> ExitCode {
    let mut pages: Option<(usize, usize)> = None;
    let mut file: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("pdftext {VERSION}");
                return ExitCode::SUCCESS;
            }
            "--pages" => match args.next() {
                Some(v) => match parse_pages(&v) {
                    Some(p) => pages = Some(p),
                    None => {
                        eprintln!("pdftext: bad page range {v:?}");
                        return ExitCode::from(2);
                    }
                },
                None => {
                    eprintln!("pdftext: --pages needs a value");
                    return ExitCode::from(2);
                }
            },
            other => file = Some(other.to_string()),
        }
    }
    let Some(file) = file else {
        eprintln!("pdftext: no input file");
        return ExitCode::from(2);
    };
    let data = match std::fs::read(&file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("pdftext: {e}");
            return ExitCode::FAILURE;
        }
    };
    let doc = match pdftext::pdf::scan_document(&data) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("pdftext: {e}");
            return ExitCode::FAILURE;
        }
    };
    let nums = doc.page_numbers();
    let (first, last) = pages.unwrap_or((1, nums.len()));
    for (i, &num) in nums.iter().enumerate() {
        let pageno = i + 1;
        if pageno < first || pageno > last {
            continue;
        }
        println!("--- page {pageno} ---");
        println!("{}", text::extract_page_text(&doc, num));
    }
    ExitCode::SUCCESS
}

fn parse_pages(v: &str) -> Option<(usize, usize)> {
    if let Some((a, b)) = v.split_once('-') {
        let a: usize = a.parse().ok()?;
        let b: usize = b.parse().ok()?;
        if a >= 1 && b >= a {
            return Some((a, b));
        }
        return None;
    }
    let n: usize = v.parse().ok()?;
    Some((n, n))
}
