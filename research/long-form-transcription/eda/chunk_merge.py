"""Reference implementation of rolling-window transcript merging.

Pure Python, stdlib only, so it can be unit-tested without a model, GPU, or
audio file. This is the algorithm `chunk_harness`'s JSON output (real Parakeet
runs) or the bundled fixtures (synthetic runs) get fed through in the EDA
notebook. See ../SPEC.md for the design this implements.

Chunk JSON schema (also produced by chunk_harness --out):

    {
      "index": 0,
      "start": 0.0,           # seconds, offset into the original recording
      "end": 30.0,
      "segments": [
        {"start": 0.0, "end": 4.8, "text": "we should discuss the budget"},
        ...
      ]
    }
"""

from __future__ import annotations

import difflib
from dataclasses import dataclass, field
from typing import Sequence

# Shortest matching run of words we trust as a genuine overlap rather than
# coincidence. Below this, we don't trust the text match and fall back to a
# timestamp-based cut instead.
MIN_MATCH_WORDS = 3


@dataclass
class Segment:
    start: float
    end: float
    text: str

    @property
    def mid(self) -> float:
        return (self.start + self.end) / 2


@dataclass
class Chunk:
    index: int
    start: float
    end: float
    segments: list[Segment]

    @classmethod
    def from_dict(cls, d: dict) -> "Chunk":
        return cls(
            index=d["index"],
            start=d["start"],
            end=d["end"],
            segments=[Segment(**s) for s in d.get("segments", [])],
        )


@dataclass
class Boundary:
    """Diagnostics for one chunk-to-chunk splice, kept for the EDA notebook
    to report boundary quality across a parameter sweep."""

    chunk_a: int
    chunk_b: int
    match_words: int
    low_confidence: bool


@dataclass
class MergedTranscript:
    text: str
    words: list[str]
    boundaries: list[Boundary] = field(default_factory=list)


def _words(segments: Sequence[Segment]) -> list[str]:
    out: list[str] = []
    for seg in segments:
        out.extend(seg.text.split())
    return out


def merge_chunks(chunks: Sequence[Chunk]) -> MergedTranscript:
    """Stitch consecutive overlapping chunks into one transcript.

    For each adjacent pair, take the segments that fall in the shared overlap
    window on both sides, find the longest run of words they agree on, and
    cut there so the duplicated overlap doesn't show up twice. Falls back to
    a timestamp cut at the overlap midpoint when the two chunks don't agree
    on any run of >= MIN_MATCH_WORDS words (flags the boundary
    low_confidence so the EDA notebook can surface it).

    Chunks are expected to be produced with a fixed window and hop (window -
    overlap) as described in SPEC.md, but this function only relies on each
    chunk's own start/end and segment timestamps -- it doesn't assume a
    uniform window/overlap across the whole recording.
    """
    if not chunks:
        return MergedTranscript(text="", words=[])

    ordered = sorted(chunks, key=lambda c: c.index)
    words = _words(ordered[0].segments)
    boundaries: list[Boundary] = []

    for prev, nxt in zip(ordered, ordered[1:]):
        overlap_start = nxt.start
        overlap_end = prev.end

        if overlap_end <= overlap_start:
            # No actual time overlap between these two chunks (hop >= window)
            # -- nothing to splice, just concatenate.
            words.extend(_words(nxt.segments))
            boundaries.append(Boundary(prev.index, nxt.index, 0, False))
            continue

        tail_segs = [s for s in prev.segments if s.mid >= overlap_start]
        head_segs = [s for s in nxt.segments if s.mid <= overlap_end]
        tail_words = _words(tail_segs)
        head_words = _words(head_segs)

        matcher = difflib.SequenceMatcher(None, tail_words, head_words, autojunk=False)
        blocks = [b for b in matcher.get_matching_blocks() if b.size > 0]
        best = max(blocks, key=lambda b: b.size, default=None)

        if best is not None and best.size >= MIN_MATCH_WORDS:
            # Keep tail up to the start of the match, then take head from the
            # start of the match onward -- the matched (duplicated) words
            # themselves are kept exactly once, sourced from head. Dropping
            # the tail suffix without re-adding the match from head would
            # silently delete real spoken content, not just the duplicate.
            drop_from_tail = len(tail_words) - best.a
            resume_from = best.b
            low_confidence = False
            match_words = best.size
        else:
            # No confident textual match -- cut at the temporal midpoint of
            # the overlap instead. Segments starting before the midpoint are
            # tail's territory (already in `words`, or kept as-is); segments
            # starting at/after the midpoint are head's territory.
            midpoint = (overlap_start + overlap_end) / 2
            drop_from_tail = len(_words([s for s in tail_segs if s.start >= midpoint]))
            resume_from = len(_words([s for s in head_segs if s.start < midpoint]))
            low_confidence = True
            match_words = 0

        if drop_from_tail > 0:
            del words[-drop_from_tail:]
        words.extend(head_words[resume_from:])
        # Anything in nxt beyond the overlap window entirely.
        words.extend(_words([s for s in nxt.segments if s.mid > overlap_end]))
        boundaries.append(Boundary(prev.index, nxt.index, match_words, low_confidence))

    return MergedTranscript(text=" ".join(words), words=words, boundaries=boundaries)
