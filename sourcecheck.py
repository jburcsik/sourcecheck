"""Flag sentences in an edited text that add words not found in its source."""

from __future__ import annotations

import argparse
import difflib
import json
import re
import sys
from pathlib import Path
from typing import TypedDict

WORD = re.compile(r"[A-Za-z0-9]+(?:'[A-Za-z]+)?")
SENTENCE_END = re.compile(r"(?<=[.!?])\s+")


class Flagged(TypedDict):
    added: int
    sentence: str


class Report(TypedDict):
    over_limit: int
    retained: float
    words_in: int
    words_out: int
    new_words: int
    flagged: list[Flagged]


def words(text: str) -> list[str]:
    """Return the words in text, lowercased, with curly apostrophes straightened."""
    return [word.lower() for word in WORD.findall(text.replace("\u2019", "'"))]


def sentences(text: str) -> list[str]:
    """Split text into sentences ending at ., ! or ? followed by whitespace."""
    return [part for part in SENTENCE_END.split(" ".join(text.split())) if part]


def added_words(original: str, edited: str) -> int:
    """Count the words in edited that are inserted or replaced relative to original."""
    matcher = difflib.SequenceMatcher(None, words(original), words(edited))
    return sum(
        j2 - j1 for tag, _, _, j1, j2 in matcher.get_opcodes() if tag in ("insert", "replace")
    )


def retained(source: str, edited: str) -> float:
    """Return the share of edited words that sit inside a 3-word run copied from source."""
    src, out = words(source), words(edited)
    runs = {tuple(src[i : i + 3]) for i in range(len(src) - 2)}
    covered = [False] * len(out)
    for i in range(len(out) - 2):
        if tuple(out[i : i + 3]) in runs:
            covered[i : i + 3] = [True] * 3
    return round(sum(covered) / max(len(out), 1), 3)


def check(source: str, edited: str, max_new: int = 3) -> Report:
    """Compare each edited sentence with its closest source sentence and summarise."""
    originals = sentences(source)
    if not originals:
        raise ValueError("source has no sentences")
    flagged: list[Flagged] = []
    for line in edited.splitlines():
        for sentence in sentences(line):
            new = min(added_words(original, sentence) for original in originals)
            if new > max_new:
                flagged.append({"added": new, "sentence": sentence})
    src, out = words(source), words(edited)
    return {
        "over_limit": len(flagged),
        "retained": retained(source, edited),
        "words_in": len(src),
        "words_out": len(out),
        "new_words": len(set(out) - set(src)),
        "flagged": flagged,
    }


def main(argv: list[str] | None = None) -> int:
    """Run the command line interface and return its exit code."""
    parser = argparse.ArgumentParser(prog="sourcecheck", description=__doc__)
    parser.add_argument("source", type=Path, help="the original text")
    parser.add_argument("edited", type=Path, help="the edited text to check")
    parser.add_argument(
        "--max-new", type=int, default=3, help="new words allowed per sentence (default 3)"
    )
    parser.add_argument("--json", action="store_true", help="print the report as JSON")
    args = parser.parse_args(argv)
    try:
        source = args.source.read_text(encoding="utf-8")
        edited = args.edited.read_text(encoding="utf-8")
        report = check(source, edited, args.max_new)
    except (OSError, ValueError) as error:
        print(f"sourcecheck: {error}", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps(report))
    else:
        for item in report["flagged"]:
            print(f"{item['added']} new words: {item['sentence']}")
        print(json.dumps({key: value for key, value in report.items() if key != "flagged"}))
    return 1 if report["flagged"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
