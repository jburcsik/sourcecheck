//! Flag sentences in an edited text that add words not found in its source.
//!
//! A port of `sourcecheck.py`. Every function mirrors its Python namesake, and
//! the text rules follow CPython (`str.split`, `str.splitlines`, `round`,
//! `json.dumps`) so both versions print the same bytes for the same input.

pub mod difflib;

use std::collections::HashSet;
use std::fmt::Write;

use std::collections::HashMap;

use difflib::{BIndex, FxBuild, SequenceMatcher, Tag};

/// One edited sentence with more new words than allowed.
#[derive(Debug, Clone, PartialEq)]
pub struct Flagged {
    pub added: usize,
    pub sentence: String,
}

/// The summary `check` returns.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub over_limit: usize,
    pub retained: f64,
    pub words_in: usize,
    pub words_out: usize,
    pub new_words: usize,
    pub flagged: Vec<Flagged>,
}

/// Python's `str.isspace`: Unicode White_Space plus the four ASCII
/// separators U+001C to U+001F.
fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Return the words in text, lowercased, with curly apostrophes straightened.
///
/// Matches the regex `[A-Za-z0-9]+(?:'[A-Za-z]+)?`.
pub fn words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text
        .chars()
        .map(|c| if c == '\u{2019}' { '\'' } else { c })
        .collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_alphanumeric() {
            i += 1;
        }
        if i + 1 < chars.len() && chars[i] == '\'' && chars[i + 1].is_ascii_alphabetic() {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
        }
        found.push(
            chars[start..i]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase(),
        );
    }
    found
}

/// Split text into sentences ending at `.`, `!` or `?` followed by whitespace.
pub fn sentences(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for token in text.split(py_isspace).filter(|t| !t.is_empty()) {
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(token);
        if token.ends_with(['.', '!', '?']) {
            result.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// Python's `str.splitlines()` without line ends.
pub fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        let is_break = matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if is_break {
            lines.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' {
                if let Some(&(_, '\n')) = iter.peek() {
                    iter.next();
                    end += 1;
                }
            }
            start = end;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Count the words in edited that are inserted or replaced relative to original.
pub fn added_words(original: &str, edited: &str) -> usize {
    added_tokens(&words(original), &words(edited))
}

/// `added_words` on word lists that are already split (words or their ids).
fn added_tokens<T: Eq + std::hash::Hash>(a: &[T], b: &[T]) -> usize {
    added_indexed(a, &BIndex::new(b))
}

/// Inserted or replaced items of `b` against `a`, with `b` already indexed.
fn added_indexed<T: Eq + std::hash::Hash>(a: &[T], b: &BIndex<T>) -> usize {
    SequenceMatcher::with_index(a, b)
        .get_opcodes()
        .into_iter()
        .filter(|(tag, ..)| matches!(tag, Tag::Insert | Tag::Replace))
        .map(|(_, _, _, j1, j2)| j2 - j1)
        .sum()
}

/// Python's `round(x, 3)`: the decimal nearest the exact binary value.
fn round3(x: f64) -> f64 {
    format!("{x:.3}").parse().expect("formatted float parses")
}

/// Return the share of edited words that sit inside a 3-word run copied from source.
pub fn retained(source: &str, edited: &str) -> f64 {
    let (src, out) = (words(source), words(edited));
    let runs: HashSet<&[String]> = src.windows(3).collect();
    let mut covered = vec![false; out.len()];
    for (i, window) in out.windows(3).enumerate() {
        if runs.contains(window) {
            covered[i..i + 3].fill(true);
        }
    }
    let hits = covered.iter().filter(|&&c| c).count();
    round3(hits as f64 / out.len().max(1) as f64)
}

/// Compare each edited sentence with its closest source sentence and summarise.
pub fn check(source: &str, edited: &str, max_new: i64) -> Result<Report, String> {
    let originals = sentences(source);
    if originals.is_empty() {
        return Err("source has no sentences".to_string());
    }
    // Words become small integer ids once, so each comparison hashes integers,
    // and each edited sentence is indexed once for all source sentences.
    let mut ids: HashMap<String, u32, FxBuild> = HashMap::default();
    let mut intern = |text: &str| -> Vec<u32> {
        words(text)
            .into_iter()
            .map(|w| {
                let next = ids.len() as u32;
                *ids.entry(w).or_insert(next)
            })
            .collect()
    };
    let original_words: Vec<Vec<u32>> = originals.iter().map(|o| intern(o)).collect();
    let mut flagged = Vec::new();
    for line in splitlines(edited) {
        for sentence in sentences(line) {
            let sentence_words = intern(&sentence);
            let index = BIndex::new(&sentence_words);
            let new = original_words
                .iter()
                .map(|o| added_indexed(o, &index))
                .min()
                .unwrap_or(0);
            if new as i64 > max_new {
                flagged.push(Flagged {
                    added: new,
                    sentence,
                });
            }
        }
    }
    let (src, out) = (words(source), words(edited));
    let src_set: HashSet<&String> = src.iter().collect();
    let out_set: HashSet<&String> = out.iter().collect();
    Ok(Report {
        over_limit: flagged.len(),
        retained: retained(source, edited),
        words_in: src.len(),
        words_out: out.len(),
        new_words: out_set.difference(&src_set).count(),
        flagged,
    })
}

/// A JSON string as Python's `json.dumps` writes it (ensure_ascii).
fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    write!(out, "\\u{unit:04x}").expect("write to String");
                }
            }
        }
    }
    out.push('"');
    out
}

/// A float as Python's `repr`: shortest round trip, always with a decimal point.
fn json_float(x: f64) -> String {
    let text = format!("{x}");
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// The summary keys in Python's order, without `flagged`.
fn summary_fields(report: &Report) -> String {
    format!(
        "\"over_limit\": {}, \"retained\": {}, \"words_in\": {}, \"words_out\": {}, \"new_words\": {}",
        report.over_limit,
        json_float(report.retained),
        report.words_in,
        report.words_out,
        report.new_words
    )
}

/// The full report as `json.dumps(report)` prints it.
pub fn report_json(report: &Report) -> String {
    let flagged: Vec<String> = report
        .flagged
        .iter()
        .map(|f| {
            format!(
                "{{\"added\": {}, \"sentence\": {}}}",
                f.added,
                json_string(&f.sentence)
            )
        })
        .collect();
    format!(
        "{{{}, \"flagged\": [{}]}}",
        summary_fields(report),
        flagged.join(", ")
    )
}

/// The summary line the default output ends with.
pub fn summary_json(report: &Report) -> String {
    format!("{{{}}}", summary_fields(report))
}

/// Python's text-mode read: `\r\n` and lone `\r` become `\n`.
pub fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str =
        "The morning train leaves the harbour at eight and reaches the hills by ten. \
        Rain is expected after noon, so bring a coat. \
        The soup needs two onions, a carrot and a pinch of salt.";
    const CLEAN: &str = "The soup needs two onions, a carrot and salt. Rain is expected after noon, so bring a coat.";
    const FOUR_NEW: &str = "Rain is expected after noon, so bring a thick warm wool winter coat.";
    const UNRELATED: &str = "A kettle whistles on the stove while the cat sleeps under the chair.";

    #[test]
    fn words_keeps_apostrophes() {
        assert_eq!(
            words("Don't stop the train"),
            ["don't", "stop", "the", "train"]
        );
    }

    #[test]
    fn words_straightens_curly_apostrophes() {
        assert_eq!(words("Don\u{2019}t stop"), ["don't", "stop"]);
    }

    #[test]
    fn words_keeps_numbers() {
        assert_eq!(
            words("Platform 9 opens at 10am"),
            ["platform", "9", "opens", "at", "10am"]
        );
    }

    #[test]
    fn words_lowercases() {
        assert_eq!(words("Rain RAIN rain"), ["rain", "rain", "rain"]);
    }

    #[test]
    fn words_punctuation_only() {
        assert!(words("... !? --").is_empty());
    }

    #[test]
    fn added_words_cases() {
        assert_eq!(added_words("bring a coat", "Bring a coat."), 0);
        assert_eq!(
            added_words("the soup needs two onions", "the soup needs onions"),
            0
        );
        assert_eq!(
            added_words("the soup needs onions", "the soup really needs two onions"),
            2
        );
        assert_eq!(added_words("bring a coat", "bring an umbrella"), 2);
        assert_eq!(added_words("salt and pepper", "pepper and salt"), 2);
    }

    #[test]
    fn check_clean_edit_passes() {
        let report = check(SOURCE, CLEAN, 3).unwrap();
        assert_eq!(report.over_limit, 0);
        assert!(report.flagged.is_empty());
    }

    #[test]
    fn check_flags_four_new_words() {
        let report = check(SOURCE, FOUR_NEW, 3).unwrap();
        assert_eq!(
            report.flagged,
            vec![Flagged {
                added: 4,
                sentence: FOUR_NEW.to_string()
            }]
        );
        assert_eq!(check(SOURCE, FOUR_NEW, 4).unwrap().over_limit, 0);
    }

    #[test]
    fn check_multiline_edited() {
        let report = check(SOURCE, &format!("{CLEAN}\n\n{FOUR_NEW}\n"), 3).unwrap();
        assert_eq!(report.over_limit, 1);
        assert_eq!(report.flagged[0].sentence, FOUR_NEW);
    }

    #[test]
    fn check_matches_later_source_sentence() {
        let report = check(
            SOURCE,
            "The soup needs two onions, a carrot and a pinch of salt.",
            3,
        )
        .unwrap();
        assert_eq!(report.over_limit, 0);
        assert_eq!(report.retained, 1.0);
    }

    #[test]
    fn check_empty_edited() {
        let report = check(SOURCE, "", 3).unwrap();
        assert_eq!(
            (report.over_limit, report.words_out, report.retained),
            (0, 0, 0.0)
        );
    }

    #[test]
    fn check_empty_source_errors() {
        assert!(check("   ", CLEAN, 3).is_err());
    }

    #[test]
    fn retained_bounds() {
        assert_eq!(retained(SOURCE, UNRELATED), 0.0);
        assert_eq!(retained(SOURCE, SOURCE), 1.0);
    }

    #[test]
    fn splitlines_follows_python() {
        assert_eq!(splitlines("a\r\nb\rc\u{2028}d\n"), ["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\n\nb"), ["a", "", "b"]);
        assert!(splitlines("").is_empty());
    }

    #[test]
    fn json_matches_python_dumps() {
        assert_eq!(
            json_string("caf\u{e9} \"q\" \u{1f426}"),
            "\"caf\\u00e9 \\\"q\\\" \\ud83d\\udc26\""
        );
        assert_eq!(json_float(1.0), "1.0");
        assert_eq!(json_float(0.125), "0.125");
    }
}
