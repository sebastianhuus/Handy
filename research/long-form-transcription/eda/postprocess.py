"""Simple, deterministic post-processing for chunk_harness/merge output.

Pure Python, stdlib only, applied to `merge_chunks`' output words -- see
../SPEC.md and real_runs/FINDINGS.md for the empirical basis. Deliberately
narrow in scope: this is testing whether cheap, rule-based cleanup alone
(no diarization, no LLM pass, nothing like the several minutes Wispr Flow's
note-taker spends) can close a meaningful chunk of the gap to a heavier
post-processed transcript, not trying to fully replicate one.
"""

from __future__ import annotations

# Cross-checked against an independent transcript (Wispr Flow) of the same
# ~46-minute recording (real_runs/FINDINGS.md): real disfluencies there
# never exceed 4 consecutive repeats of the same word anywhere in the
# lecture. Parakeet's repetition-loop artifact produces runs of 10-11+.
# MIN_RUN_TO_COLLAPSE is set just above that observed natural ceiling --
# not a principled bound, just the best evidence available from one
# recording + one cross-check transcript (see FINDINGS.md's caveats on the
# false-positive tail case this doesn't cover: a real, slow, meaningful
# repetition of a content word).
MIN_RUN_TO_COLLAPSE = 5
COLLAPSE_TO = 1


def collapse_repeated_words(
    words: list[str],
    min_run: int = MIN_RUN_TO_COLLAPSE,
    collapse_to: int = COLLAPSE_TO,
) -> list[str]:
    """Collapse runs of >=min_run consecutive identical words (case-
    insensitive comparison, original casing preserved) down to collapse_to
    occurrences. Runs shorter than min_run are left untouched -- this is
    intentionally conservative about not touching ordinary short repeated
    words ("no, no, no" is 3 and stays put at the default threshold).
    """
    out: list[str] = []
    i = 0
    n = len(words)
    while i < n:
        j = i
        while j + 1 < n and words[j + 1].lower() == words[i].lower():
            j += 1
        run_len = j - i + 1
        if run_len >= min_run:
            out.extend(words[i : i + collapse_to])
        else:
            out.extend(words[i : j + 1])
        i = j + 1
    return out
