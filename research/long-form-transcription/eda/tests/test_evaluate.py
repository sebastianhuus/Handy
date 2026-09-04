import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from evaluate import evaluate_run, normalize_reference  # noqa: E402


def write_run(tmp_path, name, chunks, **extra):
    data = {"window_s": 30.0, "overlap_s": 6.0, "input": "x.wav", "chunks": chunks, **extra}
    path = tmp_path / f"{name}.json"
    path.write_text(json.dumps(data))
    return path


def test_normalize_reference_strips_punctuation_and_lowers():
    assert normalize_reference("Hello, World! It's fine.") == ["hello", "world", "it's", "fine"]


def test_evaluate_run_without_reference_has_no_wer_keys():
    def seg(s, e, t):
        return {"start": s, "end": e, "text": t}

    chunks = [{"index": 0, "start": 0, "end": 10, "transcribe_seconds": 0.5, "segments": [seg(0, 10, "hello there friend")]}]
    import tempfile

    with tempfile.TemporaryDirectory() as d:
        p = write_run(Path(d), "run", chunks)
        result = evaluate_run(p, None, None)
    assert "wer_raw" not in result
    assert result["n_chunks"] == 1
    assert result["words_raw"] == 3


def test_evaluate_run_computes_wer_against_reference(tmp_path):
    def seg(s, e, t):
        return {"start": s, "end": e, "text": t}

    chunks = [{"index": 0, "start": 0, "end": 10, "transcribe_seconds": 0.5, "segments": [seg(0, 10, "hello there friend")]}]
    p = write_run(tmp_path, "run", chunks)
    reference_words = normalize_reference("hello there friend")
    result = evaluate_run(p, reference_words, None)
    assert result["wer_raw"] == 0.0
    assert result["wer_after_repeat_collapse"] == 0.0


def test_evaluate_run_repeat_collapse_fixes_wer(tmp_path):
    def seg(s, e, t):
        return {"start": s, "end": e, "text": t}

    # engine repeats "uh" 6x where the reference has none at all.
    chunks = [{"index": 0, "start": 0, "end": 10, "transcribe_seconds": 0.5, "segments": [seg(0, 10, "hello uh uh uh uh uh uh there friend")]}]
    p = write_run(tmp_path, "run", chunks)
    reference_words = normalize_reference("hello there friend")
    result = evaluate_run(p, reference_words, None)
    # collapse_repeated_words' default collapses a >=5-run down to 1
    # occurrence, not 0 -- "uh" survives once, so this isn't a perfect
    # match, but it's a much smaller error than the uncollapsed 6x run.
    assert result["wer_raw"] > result["wer_after_repeat_collapse"]
    assert result["wer_after_repeat_collapse"] == round(1 / 3, 4)


def test_evaluate_run_strip_fillers_ignores_uh_um(tmp_path):
    def seg(s, e, t):
        return {"start": s, "end": e, "text": t}

    chunks = [{"index": 0, "start": 0, "end": 10, "transcribe_seconds": 0.5, "segments": [seg(0, 10, "hello uh there um friend")]}]
    p = write_run(tmp_path, "run", chunks)
    reference_words = normalize_reference("hello there friend")
    result = evaluate_run(p, reference_words, None, strip_fillers=True)
    assert result["wer_raw"] == 0.0


def test_evaluate_run_empty_chunks_reports_error(tmp_path):
    p = write_run(tmp_path, "empty_run", [])
    result = evaluate_run(p, None, None)
    assert "error" in result
