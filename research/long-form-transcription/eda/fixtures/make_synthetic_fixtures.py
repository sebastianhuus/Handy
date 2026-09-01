"""Deterministic synthetic fixture generator for the EDA notebook and tests.

Simulates a rolling-window transcription run without a real model: lays a
reference script out on a fake timeline at a fixed words-per-second rate,
slices it into (window, overlap) chunks the way chunk_harness would, and
re-serializes each chunk's slice of the reference as that chunk's
"transcription" -- optionally dropping scattered words to simulate the kind
of boundary disagreement a real ASR model produces on a mid-word audio cut,
which is what should trip the merge algorithm's low-confidence fallback.

This is NOT a substitute for running chunk_harness against real Parakeet
output -- it validates the merge/metrics pipeline mechanics and gives the
notebook something to load out of the box before real audio is available.
Real model quality still has to be measured against real audio; see
../SPEC.md.

Run: python3 fixtures/make_synthetic_fixtures.py
"""

from __future__ import annotations

import json
from pathlib import Path

FIXTURES_DIR = Path(__file__).resolve().parent

REFERENCE = (
    "good morning everyone thanks for joining today we want to cover three "
    "things first the budget for next quarter second the hiring plan for "
    "the engineering team and third the timeline for the product launch "
    "lets start with the budget the current spend is on track but we "
    "expect an increase once the new hires are onboarded which brings us "
    "to the hiring plan we are looking to add four engineers this quarter "
    "two on the backend team and two on the frontend team finally the "
    "product launch is targeted for the end of the quarter pending final "
    "qa signoff"
).split()

WORDS_PER_SECOND = 2.5  # ~150 wpm, a reasonably brisk speaker


def word_timeline(words: list[str]) -> list[tuple[float, float, str]]:
    """[(start, end, word), ...] laying words back-to-back on a fake clock."""
    step = 1 / WORDS_PER_SECOND
    return [(i * step, (i + 1) * step, w) for i, w in enumerate(words)]


def make_run(window_s: float, overlap_s: float, drop_every: int | None) -> list[dict]:
    """Chunk the reference timeline into overlapping windows, packaged as
    chunk_harness-shaped dicts (one segment per word). With drop_every set,
    every Nth word of chunks after the first is omitted from that chunk's
    transcription -- simulating a model that garbles words differently each
    time it's handed a slightly different slice of audio at a boundary.
    """
    timeline = word_timeline(REFERENCE)
    total_end = timeline[-1][1]
    hop = window_s - overlap_s
    assert hop > 0, "overlap must be smaller than window"

    chunks: list[dict] = []
    start = 0.0
    index = 0
    while True:
        end = min(start + window_s, total_end)
        words_in_chunk = [w for w in timeline if start <= w[0] < end]

        segments = []
        for i, (w_start, w_end, word) in enumerate(words_in_chunk):
            if drop_every and index > 0 and i % drop_every == 0:
                continue  # simulate a mis-heard/dropped word near a re-cut boundary
            segments.append({"start": w_start, "end": w_end, "text": word})

        chunks.append({"index": index, "start": start, "end": end, "segments": segments})

        if end >= total_end:
            break
        start += hop
        index += 1

    return chunks


def main() -> None:
    (FIXTURES_DIR / "reference.txt").write_text(" ".join(REFERENCE) + "\n")

    configs = [
        # (name, window_s, overlap_s, drop_every)
        ("window30_overlap6_clean", 30.0, 6.0, None),
        ("window30_overlap6_noisy", 30.0, 6.0, 3),
        ("window12_overlap2_noisy", 12.0, 2.0, 3),
    ]
    for name, window_s, overlap_s, drop_every in configs:
        chunks = make_run(window_s, overlap_s, drop_every)
        out = {"window_s": window_s, "overlap_s": overlap_s, "chunks": chunks}
        (FIXTURES_DIR / f"{name}.json").write_text(json.dumps(out, indent=2) + "\n")
        print(f"wrote {name}.json ({len(chunks)} chunks)")


if __name__ == "__main__":
    main()
