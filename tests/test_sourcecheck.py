import json
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

import pytest

from sourcecheck import added_words, check, main, retained, words

SOURCE = (
    "The morning train leaves the harbour at eight and reaches the hills by ten. "
    "Rain is expected after noon, so bring a coat. "
    "The soup needs two onions, a carrot and a pinch of salt."
)
CLEAN = (
    "The soup needs two onions, a carrot and salt. Rain is expected after noon, so bring a coat."
)
FOUR_NEW = "Rain is expected after noon, so bring a thick warm wool winter coat."
UNRELATED = "A kettle whistles on the stove while the cat sleeps under the chair."


@pytest.fixture
def paths(tmp_path: Path) -> Callable[[str, str], list[str]]:
    def write(source: str, edited: str) -> list[str]:
        files = [tmp_path / "source.txt", tmp_path / "edited.txt"]
        for path, text in zip(files, (source, edited)):
            path.write_text(text, encoding="utf-8")
        return [str(path) for path in files]

    return write


def test_words_keeps_apostrophes() -> None:
    assert words("Don't stop the train") == ["don't", "stop", "the", "train"]


def test_words_straightens_curly_apostrophes() -> None:
    assert words("Don\u2019t stop") == ["don't", "stop"]


def test_words_keeps_numbers() -> None:
    assert words("Platform 9 opens at 10am") == ["platform", "9", "opens", "at", "10am"]


def test_words_lowercases() -> None:
    assert words("Rain RAIN rain") == ["rain", "rain", "rain"]


def test_words_punctuation_only() -> None:
    assert words("... !? --") == []


def test_added_words_identical() -> None:
    assert added_words("bring a coat", "Bring a coat.") == 0


def test_added_words_deletion() -> None:
    assert added_words("the soup needs two onions", "the soup needs onions") == 0


def test_added_words_insertion() -> None:
    assert added_words("the soup needs onions", "the soup really needs two onions") == 2


def test_added_words_replacement() -> None:
    assert added_words("bring a coat", "bring an umbrella") == 2


def test_added_words_reordering() -> None:
    assert added_words("salt and pepper", "pepper and salt") == 2


def test_check_clean_edit_passes() -> None:
    result = check(SOURCE, CLEAN)
    assert result["over_limit"] == 0
    assert result["flagged"] == []


def test_check_flags_four_new_words() -> None:
    result = check(SOURCE, FOUR_NEW)
    assert result["over_limit"] == 1
    assert result["flagged"] == [{"added": 4, "sentence": FOUR_NEW}]


def test_check_max_new_four_passes() -> None:
    assert check(SOURCE, FOUR_NEW, max_new=4)["over_limit"] == 0


def test_check_multiline_edited() -> None:
    edited = f"{CLEAN}\n\n{FOUR_NEW}\n"
    result = check(SOURCE, edited)
    assert result["over_limit"] == 1
    assert result["flagged"][0]["sentence"] == FOUR_NEW


def test_check_matches_later_source_sentence() -> None:
    result = check(SOURCE, "The soup needs two onions, a carrot and a pinch of salt.")
    assert result["over_limit"] == 0
    assert result["retained"] == 1.0


def test_check_empty_edited() -> None:
    result = check(SOURCE, "")
    assert result["over_limit"] == 0
    assert result["words_out"] == 0
    assert result["retained"] == 0.0


def test_check_empty_source_raises() -> None:
    with pytest.raises(ValueError):
        check("   ", CLEAN)


def test_retained_unrelated_text() -> None:
    assert retained(SOURCE, UNRELATED) == 0.0


def test_retained_identical_text() -> None:
    assert retained(SOURCE, SOURCE) == 1.0


def test_main_exit_zero_and_summary_line(paths, capsys) -> None:
    assert main(paths(SOURCE, CLEAN)) == 0
    summary = json.loads(capsys.readouterr().out.strip().splitlines()[-1])
    assert summary["over_limit"] == 0
    assert summary["words_in"] == len(words(SOURCE))


def test_main_exit_one_when_flagged(paths, capsys) -> None:
    assert main(paths(SOURCE, FOUR_NEW)) == 1
    out = capsys.readouterr().out
    assert out.startswith(f"4 new words: {FOUR_NEW}\n")


def test_main_max_new_option(paths) -> None:
    assert main(paths(SOURCE, FOUR_NEW) + ["--max-new", "4"]) == 0


def test_main_missing_file_exits_two(paths, capsys) -> None:
    source, edited = paths(SOURCE, CLEAN)
    assert main([source, edited + ".missing"]) == 2
    assert capsys.readouterr().err


def test_main_empty_source_exits_two(paths, capsys) -> None:
    assert main(paths("", CLEAN)) == 2
    assert capsys.readouterr().err


def test_main_json_output(paths, capsys) -> None:
    assert main(paths(SOURCE, FOUR_NEW) + ["--json"]) == 1
    report = json.loads(capsys.readouterr().out)
    assert report["over_limit"] == 1
    assert report["flagged"][0]["added"] == 4


def test_module_runs_as_script(paths) -> None:
    command = [sys.executable, "-m", "sourcecheck", *paths(SOURCE, FOUR_NEW)]
    run = subprocess.run(
        command, check=False, cwd=Path(__file__).parents[1], capture_output=True, text=True
    )
    assert run.returncode == 1
    assert "4 new words" in run.stdout
