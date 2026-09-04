"""Pareto-frontier analysis for the window/overlap (or engine) sweep.

Pure Python, stdlib only, same convention as chunk_merge.py/postprocess.py/
wer.py. The one tradeoff this research keeps coming back to: transcript
quality (WER, lower is better) vs. throughput (realtime factor, higher is
better). A config is Pareto-optimal if no other config in the same sweep
beats it on *both* axes at once -- those are the only configs worth
actually considering; anything dominated is strictly worse than some
alternative on offer, for both quality and speed.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ParetoPoint:
    label: str
    wer: float
    realtime_factor: float


def pareto_frontier(points: list[ParetoPoint]) -> list[ParetoPoint]:
    """Returns the subset of `points` not dominated by any other point.

    Point A dominates point B if A.wer <= B.wer and A.realtime_factor >=
    B.realtime_factor, with at least one strict inequality (minimize wer,
    maximize realtime_factor) -- i.e. A is at least as good on both axes
    and strictly better on one. Points that tie exactly with another point
    don't dominate each other, so both are kept.

    O(n^2), which is fine -- a sweep is a handful of configs, not thousands.
    """
    frontier = []
    for p in points:
        dominated = any(
            other is not p and other.wer <= p.wer and other.realtime_factor >= p.realtime_factor and (other.wer < p.wer or other.realtime_factor > p.realtime_factor)
            for other in points
        )
        if not dominated:
            frontier.append(p)
    return frontier
