//! Command line interface, matching `python -m sourcecheck`.

use std::fs;
use std::process::ExitCode;

use sourcecheck::{check, report_json, summary_json, universal_newlines};

const USAGE: &str = "usage: sourcecheck [-h] [--max-new MAX_NEW] [--json] source edited";
const HELP: &str = "\
usage: sourcecheck [-h] [--max-new MAX_NEW] [--json] source edited

Flag sentences in an edited text that add words not found in its source.

positional arguments:
  source             the original text
  edited             the edited text to check

options:
  -h, --help         show this help message and exit
  --max-new MAX_NEW  new words allowed per sentence (default 3)
  --json             print the report as JSON";

struct Args {
    source: String,
    edited: String,
    max_new: i64,
    json: bool,
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("{USAGE}\nsourcecheck: error: {message}");
    ExitCode::from(2)
}

fn parse(argv: &[String]) -> Result<Args, ExitCode> {
    let (mut positional, mut max_new, mut json) = (Vec::new(), 3i64, false);
    let mut iter = argv.iter();
    let mut options_done = false;
    while let Some(arg) = iter.next() {
        if options_done || arg == "-" || !arg.starts_with('-') || arg.parse::<f64>().is_ok() {
            positional.push(arg.clone());
            continue;
        }
        let value = match arg.as_str() {
            "--" => {
                options_done = true;
                continue;
            }
            "-h" | "--help" => {
                println!("{HELP}");
                return Err(ExitCode::SUCCESS);
            }
            "--json" => {
                json = true;
                continue;
            }
            "--max-new" => match iter.next() {
                Some(v) => v.clone(),
                None => return Err(usage_error("argument --max-new: expected one argument")),
            },
            other => match other.strip_prefix("--max-new=") {
                Some(v) => v.to_string(),
                None => return Err(usage_error(&format!("unrecognized arguments: {other}"))),
            },
        };
        max_new = value.trim().parse().map_err(|_| {
            usage_error(&format!("argument --max-new: invalid int value: '{value}'"))
        })?;
    }
    match positional.len() {
        0 => Err(usage_error(
            "the following arguments are required: source, edited",
        )),
        1 => Err(usage_error("the following arguments are required: edited")),
        2 => {
            let edited = positional.pop().expect("two items");
            let source = positional.pop().expect("one item");
            Ok(Args {
                source,
                edited,
                max_new,
                json,
            })
        }
        _ => Err(usage_error(&format!(
            "unrecognized arguments: {}",
            positional[2..].join(" ")
        ))),
    }
}

fn read(path: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|e| format!("{path}: invalid UTF-8: {e}"))?;
    Ok(universal_newlines(&text))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&argv) {
        Ok(args) => args,
        Err(code) => return code,
    };
    let report = read(&args.source)
        .and_then(|source| read(&args.edited).map(|edited| (source, edited)))
        .and_then(|(source, edited)| check(&source, &edited, args.max_new));
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            eprintln!("sourcecheck: {error}");
            return ExitCode::from(2);
        }
    };
    if args.json {
        println!("{}", report_json(&report));
    } else {
        for item in &report.flagged {
            println!("{} new words: {}", item.added, item.sentence);
        }
        println!("{}", summary_json(&report));
    }
    if report.flagged.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
