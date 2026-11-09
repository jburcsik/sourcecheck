//! Command line interface, matching `python -m sourcecheck` and its argparse
//! rules: options in any position, unambiguous prefixes (`--max 5`, `--js`),
//! `--` to end options, and negative numbers taken as values.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
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

const LONG_OPTIONS: [&str; 3] = ["--help", "--max-new", "--json"];

struct Args {
    source: PathBuf,
    edited: PathBuf,
    max_new: i64,
    json: bool,
}

/// Why parsing stopped: help was printed (exit 0) or a usage error (exit 2).
enum Stop {
    Help,
    Usage(String),
}

/// argparse's test for an argument that is a value, not an option: `-` alone,
/// a negative number, or text containing a space.
fn is_positional(arg: &str) -> bool {
    if !arg.starts_with('-') || arg == "-" {
        return true;
    }
    let rest = &arg[1..];
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let negative_number = digits(rest)
        || rest
            .split_once('.')
            .is_some_and(|(int, frac)| (int.is_empty() || digits(int)) && digits(frac));
    negative_number || arg.contains(' ')
}

/// The option an argument names, resolving unambiguous `--` prefixes, plus
/// any `=value` part. `None` for an unknown option.
fn resolve(arg: &str) -> Option<(&'static str, Option<&str>)> {
    if arg == "-h" {
        return Some(("--help", None));
    }
    let (name, value) = match arg.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (arg, None),
    };
    if name.len() < 3 || !name.starts_with("--") {
        return None;
    }
    let matches: Vec<&'static str> = LONG_OPTIONS
        .into_iter()
        .filter(|o| o.starts_with(name))
        .collect();
    match matches.as_slice() {
        [option] => Some((option, value)),
        _ => None,
    }
}

/// Python's `int()` on a decimal string: surrounding whitespace, an optional
/// sign, and single underscores between digits. Values past the i64 range
/// clamp, which compares the same way against any word count.
fn python_int(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    let mut value: i64 = 0;
    for byte in digits.bytes().filter(|&b| b != b'_') {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(byte - b'0'));
    }
    Some(if negative { -value } else { value })
}

fn parse(argv: Vec<OsString>) -> Result<Args, Stop> {
    let (mut positional, mut unknown) = (Vec::new(), Vec::new());
    let (mut max_new, mut json, mut options_done) = (3i64, false, false);
    let mut iter = argv.into_iter().peekable();
    while let Some(raw) = iter.next() {
        let arg = raw.to_string_lossy().into_owned();
        if options_done || is_positional(&arg) {
            positional.push(raw);
            continue;
        }
        if arg == "--" {
            options_done = true;
            continue;
        }
        let Some((option, inline)) = resolve(&arg) else {
            unknown.push(arg);
            continue;
        };
        match option {
            "--help" | "--json" if inline.is_some() => {
                return Err(Stop::Usage(format!(
                    "argument {option}: ignored explicit argument '{}'",
                    inline.unwrap_or("")
                )));
            }
            "--help" => return Err(Stop::Help),
            "--json" => json = true,
            _ => {
                let value = match inline {
                    Some(value) => value.to_string(),
                    None => match iter.peek().map(|next| next.to_string_lossy().into_owned()) {
                        Some(next) if is_positional(&next) => {
                            iter.next();
                            next
                        }
                        _ => {
                            return Err(Stop::Usage(
                                "argument --max-new: expected one argument".into(),
                            ))
                        }
                    },
                };
                max_new = python_int(&value).ok_or_else(|| {
                    Stop::Usage(format!("argument --max-new: invalid int value: '{value}'"))
                })?;
            }
        }
    }
    let missing = ["source", "edited"]
        .get(positional.len().min(2)..)
        .unwrap_or_default();
    if !missing.is_empty() {
        return Err(Stop::Usage(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        )));
    }
    unknown.extend(
        positional
            .drain(2..)
            .map(|p| p.to_string_lossy().into_owned()),
    );
    if !unknown.is_empty() {
        return Err(Stop::Usage(format!(
            "unrecognized arguments: {}",
            unknown.join(" ")
        )));
    }
    let edited = PathBuf::from(positional.pop().expect("two positionals"));
    let source = PathBuf::from(positional.pop().expect("two positionals"));
    Ok(Args {
        source,
        edited,
        max_new,
        json,
    })
}

fn read(path: &PathBuf) -> Result<String, String> {
    let shown = path.display();
    let bytes = fs::read(path).map_err(|e| format!("{shown}: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|e| format!("{shown}: invalid UTF-8: {e}"))?;
    Ok(universal_newlines(&text))
}

fn main() -> ExitCode {
    let args = match parse(std::env::args_os().skip(1).collect()) {
        Ok(args) => args,
        Err(Stop::Help) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(Stop::Usage(message)) => {
            eprintln!("{USAGE}\nsourcecheck: error: {message}");
            return ExitCode::from(2);
        }
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
    let mut out = String::new();
    if args.json {
        out.push_str(&report_json(&report));
        out.push('\n');
    } else {
        for item in &report.flagged {
            out.push_str(&format!("{} new words: {}\n", item.added, item.sentence));
        }
        out.push_str(&summary_json(&report));
        out.push('\n');
    }
    // A closed pipe (`sourcecheck a b | head -1`) is not an error worth a panic.
    let _ = io::stdout().lock().write_all(out.as_bytes());
    if report.flagged.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_resolve_like_argparse() {
        assert_eq!(resolve("--max"), Some(("--max-new", None)));
        assert_eq!(resolve("--m=5"), Some(("--max-new", Some("5"))));
        assert_eq!(resolve("--j"), Some(("--json", None)));
        assert_eq!(resolve("--he"), Some(("--help", None)));
        assert_eq!(resolve("--bogus"), None);
        assert_eq!(resolve("--"), None);
    }

    #[test]
    fn values_follow_argparse() {
        assert!(
            is_positional("-5")
                && is_positional("-.5")
                && is_positional("-x y")
                && is_positional("-")
        );
        assert!(!is_positional("-inf") && !is_positional("--json"));
    }

    #[test]
    fn ints_follow_python() {
        assert_eq!(python_int(" 1_0 "), Some(10));
        assert_eq!(python_int("+7"), Some(7));
        assert_eq!(python_int("-0"), Some(0));
        assert_eq!(python_int("99999999999999999999"), Some(i64::MAX));
        assert_eq!(python_int("1__0"), None);
        assert_eq!(python_int("_1"), None);
        assert_eq!(python_int("x"), None);
    }
}
