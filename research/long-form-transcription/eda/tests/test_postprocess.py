import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from postprocess import collapse_repeated_words  # noqa: E402


def test_short_runs_are_left_alone():
    words = "no no no that's wrong".split()
    assert collapse_repeated_words(words) == words


def test_long_run_is_collapsed_to_one():
    words = "I don't wanna uh uh uh uh uh uh uh uh uh uh uh pester".split()
    assert collapse_repeated_words(words) == "I don't wanna uh pester".split()


def test_case_insensitive_match_keeps_original_casing_of_survivor():
    words = "You You you You you you you you you you know".split()
    assert collapse_repeated_words(words) == "You know".split()


def test_run_exactly_at_threshold_is_collapsed():
    words = ("the " * 5 + "cat").split()
    assert collapse_repeated_words(words, min_run=5, collapse_to=1) == ["the", "cat"]


def test_run_one_below_threshold_is_untouched():
    words = ("the " * 4 + "cat").split()
    assert collapse_repeated_words(words, min_run=5) == words


def test_no_repeats_is_unchanged():
    words = "the quick brown fox jumps".split()
    assert collapse_repeated_words(words) == words


def test_empty_input():
    assert collapse_repeated_words([]) == []


def test_collapse_to_two_keeps_first_two_of_run():
    words = "you you you you you know".split()
    assert collapse_repeated_words(words, min_run=5, collapse_to=2) == "you you know".split()
