"""Word error rate: standard Levenshtein edit distance over word tokens.

Used by the EDA notebook to score a merged transcript against a manually
transcribed reference, when one is available for a test recording.
"""

from __future__ import annotations


def word_error_rate(reference: str, hypothesis: str) -> float:
    """(substitutions + insertions + deletions) / len(reference words).

    This is the textbook ASR WER definition. Case- and punctuation-sensitive
    by design -- callers should normalize both strings first (lowercase,
    strip punctuation) if that's not the comparison they want.
    """
    ref = reference.split()
    hyp = hypothesis.split()
    if not ref:
        return 0.0 if not hyp else 1.0

    prev = list(range(len(hyp) + 1))
    for i in range(1, len(ref) + 1):
        curr = [i] + [0] * len(hyp)
        for j in range(1, len(hyp) + 1):
            cost = 0 if ref[i - 1] == hyp[j - 1] else 1
            curr[j] = min(
                prev[j] + 1,  # deletion from reference
                curr[j - 1] + 1,  # insertion into hypothesis
                prev[j - 1] + cost,  # match or substitution
            )
        prev = curr
    return prev[-1] / len(ref)
