import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from wer import word_error_rate  # noqa: E402


def test_identical_strings_zero_wer():
    assert word_error_rate("the quick brown fox", "the quick brown fox") == 0.0


def test_empty_reference_and_hypothesis():
    assert word_error_rate("", "") == 0.0


def test_empty_reference_nonempty_hypothesis_is_one():
    assert word_error_rate("", "hello world") == 1.0


def test_single_substitution():
    # 1 error / 4 reference words
    assert word_error_rate("the quick brown fox", "the slow brown fox") == 0.25


def test_single_insertion():
    # hypothesis has one extra word -> 1 error / 4 reference words
    assert word_error_rate("the quick brown fox", "the quick very brown fox") == 0.25


def test_single_deletion():
    # hypothesis missing one word -> 1 error / 4 reference words
    assert word_error_rate("the quick brown fox", "the quick fox") == 0.25


def test_completely_different_same_length():
    assert word_error_rate("one two three", "four five six") == 1.0
