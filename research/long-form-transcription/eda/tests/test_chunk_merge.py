import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from chunk_merge import Chunk, Segment, merge_chunks  # noqa: E402


def make_chunk(index, start, end, segs):
    return Chunk(
        index=index,
        start=start,
        end=end,
        segments=[Segment(s, e, t) for s, e, t in segs],
    )


def test_single_chunk_passthrough():
    chunk = make_chunk(0, 0, 30, [(0, 10, "hello there friend")])
    result = merge_chunks([chunk])
    assert result.text == "hello there friend"
    assert result.boundaries == []


def test_no_chunks_returns_empty():
    result = merge_chunks([])
    assert result.text == ""
    assert result.words == []


def test_no_time_overlap_is_plain_concatenation():
    # window == hop: back-to-back chunks with no shared audio.
    a = make_chunk(0, 0, 30, [(0, 30, "the first half of the story")])
    b = make_chunk(1, 30, 60, [(30, 60, "and the second half follows")])
    result = merge_chunks([a, b])
    assert result.text == "the first half of the story and the second half follows"
    assert result.boundaries[0].low_confidence is False
    assert result.boundaries[0].match_words == 0


def test_exact_duplicate_overlap_is_deduped():
    # window=30, overlap=6, hop=24 -> chunk1 starts at 24, chunk0 ends at 30.
    a = make_chunk(
        0,
        0,
        30,
        [
            (0, 10, "we should discuss the quarterly budget"),
            (22, 30, "for next quarter and hiring"),
        ],
    )
    b = make_chunk(
        1,
        24,
        54,
        [
            (24, 32, "for next quarter and hiring plans"),
            (32, 54, "look strong this year"),
        ],
    )
    result = merge_chunks([a, b])
    assert result.text == (
        "we should discuss the quarterly budget "
        "for next quarter and hiring plans look strong this year"
    )
    boundary = result.boundaries[0]
    assert boundary.low_confidence is False
    assert boundary.match_words == 5  # "for next quarter and hiring"


def test_disagreeing_overlap_falls_back_to_timestamp_cut():
    a = make_chunk(
        0,
        0,
        30,
        [
            (0, 10, "we should discuss the quarterly budget"),
            (22, 30, "for next quarter and hiring"),
        ],
    )
    b = make_chunk(
        1,
        24,
        54,
        [
            (24, 32, "completely unrelated words here now"),
            (32, 54, "the rest continues normally"),
        ],
    )
    result = merge_chunks([a, b])
    boundary = result.boundaries[0]
    assert boundary.low_confidence is True
    assert boundary.match_words == 0
    # Both sides' overlap segments start before the (27s) midpoint, so the
    # tail (chunk 0's own words) wins the overlap and the disagreeing head
    # segment is dropped rather than duplicated -- the failure mode this
    # falls back to is losing text at a bad boundary, not doubling it.
    assert result.text == (
        "we should discuss the quarterly budget for next quarter and hiring "
        "the rest continues normally"
    )


def test_empty_chunk_does_not_crash_and_is_skipped_cleanly():
    a = make_chunk(0, 0, 30, [(0, 30, "opening remarks about the agenda")])
    b = make_chunk(1, 24, 40, [])  # e.g. VAD trimmed this whole window to silence
    c = make_chunk(2, 34, 64, [(34, 64, "closing remarks about next steps")])
    result = merge_chunks([a, b, c])
    assert "opening remarks" in result.text
    assert "closing remarks about next steps" in result.text
    assert len(result.boundaries) == 2
    assert all(isinstance(b.low_confidence, bool) for b in result.boundaries)


def test_from_dict_round_trip():
    d = {
        "index": 0,
        "start": 0.0,
        "end": 30.0,
        "segments": [{"start": 0.0, "end": 4.8, "text": "hello world"}],
    }
    chunk = Chunk.from_dict(d)
    assert chunk.index == 0
    assert chunk.segments[0].text == "hello world"
    result = merge_chunks([chunk])
    assert result.text == "hello world"
