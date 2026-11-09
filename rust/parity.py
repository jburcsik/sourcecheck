"""Run the Python and Rust CLIs on the same inputs and require identical stdout and exit codes.

Usage: python3 rust/parity.py [path-to-rust-binary] [--cases N]

Two parts:
- Texts: a fixed corpus of edge cases plus seeded random texts, including
  sentences of 200+ words that switch on difflib's autojunk rule. Python is
  called in process (the same main() the console script runs) for speed; its
  stdout is compared as UTF-8 bytes with the binary's.
- Arguments: a table of command lines (prefixes, unknown options, help,
  negative and underscored numbers, odd file names) run through both programs
  as subprocesses, comparing raw stdout bytes and exit codes.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import sourcecheck  # noqa: E402

VOCAB = (
    "the a and of to rain coat train harbour hills soup onions carrot salt pepper "
    "noon morning eight ten bring needs two pinch expected after leaves reaches "
    "don't it's we'll o'clock 9 10am 2026 Thick WARM wool winter kettle stove cat"
).split()
PUNCT = [".", ".", ".", "!", "?", ",", ";", ":", "...", "?!", ""]
SPACES = [" ", " ", " ", "  ", "\t", "\n", "\r\n", "\r", " ", " ", "　",
          " ", " ", "\u0085", "\x0b", "\x0c", "\x1c", "\x1f"]
EXTRAS = ["’", "“", "”", "café", "\U0001f426", "naïve", "\"", "\\",
          "\x07", "\x7f", "﻿", "it’s", "مرحبا"]

FIXED = [
    ("", "x."), ("   ", "a."), ("One.", ""), ("One.", "One."), ("A b c.", "\n\n\n"),
    ("Salt and pepper.", "Pepper and salt."),
    ("The cat sat. The dog ran!", "The cat sat on the very warm mat today. Why?"),
    ("Line one.\r\nLine two.\rLine three.", "Line one.\r\nLine two plus five more extra new words.\r"),
    ("Don’t stop.", "Don't stop now, please, for anything at all."),
    ("Café au lait.", "Café au lait with \U0001f426 and \"quotes\" and \\slashes\\ added here."),
    ("No terminal punctuation here", "No terminal punctuation here either way at all"),
]


def random_text(rng: random.Random, n_words: int) -> str:
    parts = []
    for _ in range(n_words):
        word = rng.choice(VOCAB) if rng.random() > 0.05 else rng.choice(EXTRAS)
        parts.append(word + (rng.choice(PUNCT) if rng.random() < 0.15 else ""))
        parts.append(rng.choice(SPACES) if rng.random() < 0.2 else " ")
    return "".join(parts)


def mutate(rng: random.Random, text: str) -> str:
    tokens = text.split(" ")
    for _ in range(rng.randint(0, 6)):
        op = rng.random()
        if op < 0.4 and tokens:
            tokens.insert(rng.randrange(len(tokens) + 1), rng.choice(VOCAB))
        elif op < 0.7 and tokens:
            tokens.pop(rng.randrange(len(tokens)))
        elif tokens:
            tokens[rng.randrange(len(tokens))] = rng.choice(VOCAB + EXTRAS)
    return " ".join(tokens)


def cases(n: int, seed: int):
    for source, edited in FIXED:
        yield source, edited, []
    rng = random.Random(seed)
    for k in range(n):
        size = rng.choice([3, 8, 20, 60, 150, 260, 400])
        source = random_text(rng, size)
        edited = mutate(rng, source) if rng.random() < 0.7 else random_text(rng, size)
        if k % 10 == 0:
            # autojunk depends on the length of one edited sentence, so build
            # sentences of 200+ words with no sentence-ending punctuation.
            def long_sentence() -> str:
                return " ".join(rng.choice(VOCAB) for _ in range(rng.randint(200, 450))) + "."
            source = " ".join(long_sentence() for _ in range(rng.randint(1, 3)))
            edited = mutate(rng, source) if rng.random() < 0.8 else long_sentence()
        flags = []
        if k % 5 == 0:
            flags.append("--json")
        if k % 7 == 0:
            flags += rng.choice([["--max-new", "0"], ["--max-new=1"], ["--max-new", "-1"], ["--max-new", "10"]])
        yield source, edited, flags


SOURCE_TEXT = "Rain is expected after noon, so bring a coat. The soup needs two onions."
FOUR_NEW_EDIT = "Rain is expected after noon, so bring a thick warm wool winter coat."
S, E = "source.txt", "edited.txt"
ARGV_TABLE = [
    [S, E], [S, E, "--json"], [S, E, "--max-new", "4"], [S, E, "--max-new=4"],
    [S, E, "--max", "5"], [S, E, "--max=5"], [S, E, "--m", "9"], [S, E, "--js"], [S, E, "--j"],
    ["--json", S, E], [S, "--json", E], [S, E, "--json", "--json"],
    ["-h"], ["--help"], ["--h"], ["--he"], [S, E, "--bogus", "--help"], [S, E, "--help", "--bogus"],
    [S, E, "--bogus"], [S, E, "extra"], [S], [],
    [S, E, "--max-new"], [S, E, "--max-new", "--json"], [S, E, "--json=1"], [S, E, "--help=1"],
    [S, E, "--max-new", "x"], [S, E, "--max-new", "1_0"], [S, E, "--max-new", "1__0"],
    [S, E, "--max-new", "99999999999999999999"], [S, E, "--max-new", "-1"], [S, E, "--max-new", "+9"],
    [S, E, "--max-new", " 5 "], [S, E, "--max-new=-0"],
    ["--", S, E], [S, "--", E], [S, E, "--", "--json"],
    [S, "-x y"], [S, "-5"], [S, "-inf"], [S, E + ".missing"], [S, "."],
]


def run_python(argv: list[str]) -> tuple[str, int]:
    out = io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
        try:
            code = sourcecheck.main(argv)
        except SystemExit as exit_:
            code = exit_.code if isinstance(exit_.code, int) else 2
    return out.getvalue(), code


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", nargs="?", default=str(ROOT / "rust/target/release/sourcecheck"))
    parser.add_argument("--cases", type=int, default=2000)
    parser.add_argument("--seed", type=int, default=7)
    args = parser.parse_args()
    failures = total = 0
    with tempfile.TemporaryDirectory() as tmp:
        src, edt = Path(tmp, "source.txt"), Path(tmp, "edited.txt")
        for source, edited, flags in cases(args.cases, args.seed):
            src.write_bytes(source.encode("utf-8"))
            edt.write_bytes(edited.encode("utf-8"))
            argv = [str(src), str(edt), *flags]
            expected = run_python(argv)
            run = subprocess.run([args.binary, *argv], capture_output=True)
            got = (run.stdout, run.returncode)
            expected = (expected[0].encode("utf-8"), expected[1])
            total += 1
            if got != expected:
                failures += 1
                if failures <= 5:
                    print(f"MISMATCH flags={flags}\n source={source!r}\n edited={edited!r}\n"
                          f" python={expected!r}\n rust  ={got!r}\n")
        Path(tmp, "-x y").write_text(FOUR_NEW_EDIT, encoding="utf-8")
        Path(tmp, "-5").write_text(FOUR_NEW_EDIT, encoding="utf-8")
        src.write_text(SOURCE_TEXT, encoding="utf-8")
        edt.write_text(FOUR_NEW_EDIT, encoding="utf-8")
        env = {**os.environ, "COLUMNS": "80", "NO_COLOR": "1"}
        for argv in ARGV_TABLE:
            python = subprocess.run([sys.executable, str(ROOT / "sourcecheck.py"), *argv],
                                    capture_output=True, cwd=tmp, env=env)
            rust = subprocess.run([str(Path(args.binary).resolve()), *argv],
                                  capture_output=True, cwd=tmp, env=env)
            total += 1
            if (python.stdout, python.returncode) != (rust.stdout, rust.returncode):
                failures += 1
                print(f"ARGV MISMATCH {argv}: python {python.returncode} {python.stdout[:80]!r}, "
                      f"rust {rust.returncode} {rust.stdout[:80]!r}")
    print(f"parity: {total - failures}/{total} cases identical")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
