"""Run the Python and Rust CLIs on the same inputs and require identical stdout and exit codes.

Usage: python3 rust/parity.py [path-to-rust-binary] [--cases N]

The Python side is called in process (the same main() the console script runs);
the Rust side runs as a subprocess. Inputs are a fixed corpus of edge cases plus
seeded random texts, so a failure is reproducible.
"""

from __future__ import annotations

import argparse
import contextlib
import io
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
        size = rng.choice([3, 8, 20, 60, 150, 260, 400])  # 200+ exercises difflib's autojunk
        source = random_text(rng, size)
        edited = mutate(rng, source) if rng.random() < 0.7 else random_text(rng, size)
        flags = []
        if k % 5 == 0:
            flags.append("--json")
        if k % 7 == 0:
            flags += rng.choice([["--max-new", "0"], ["--max-new=1"], ["--max-new", "-1"], ["--max-new", "10"]])
        yield source, edited, flags


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
            got = (run.stdout.decode("utf-8"), run.returncode)
            total += 1
            if got != expected:
                failures += 1
                if failures <= 5:
                    print(f"MISMATCH flags={flags}\n source={source!r}\n edited={edited!r}\n"
                          f" python={expected!r}\n rust  ={got!r}\n")
        for argv in (["--max-new", "x", str(src), str(edt)], [str(src)], [str(src), str(edt), "extra"],
                     [str(src), str(edt) + ".missing"]):
            expected, got = run_python(argv)[1], subprocess.run([args.binary, *argv], capture_output=True).returncode
            total += 1
            if expected != got:
                failures += 1
                print(f"EXIT MISMATCH {argv}: python {expected}, rust {got}")
    print(f"parity: {total - failures}/{total} cases identical")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
