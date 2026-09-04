import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from pareto import ParetoPoint, pareto_frontier  # noqa: E402


def test_single_point_is_always_on_frontier():
    p = ParetoPoint("a", wer=0.3, realtime_factor=10.0)
    assert pareto_frontier([p]) == [p]


def test_empty_input():
    assert pareto_frontier([]) == []


def test_clearly_dominated_point_is_excluded():
    # b has both worse WER (higher) and worse throughput (lower) than a --
    # nobody should ever pick b.
    a = ParetoPoint("a", wer=0.30, realtime_factor=20.0)
    b = ParetoPoint("b", wer=0.40, realtime_factor=10.0)
    assert pareto_frontier([a, b]) == [a]


def test_genuine_tradeoff_keeps_both_points():
    # a is more accurate but slower; b is faster but less accurate --
    # neither dominates the other, both are legitimate choices.
    a = ParetoPoint("a", wer=0.30, realtime_factor=10.0)
    b = ParetoPoint("b", wer=0.40, realtime_factor=30.0)
    result = pareto_frontier([a, b])
    assert set(result) == {a, b}


def test_exact_tie_keeps_both_points():
    a = ParetoPoint("a", wer=0.35, realtime_factor=20.0)
    b = ParetoPoint("b", wer=0.35, realtime_factor=20.0)
    result = pareto_frontier([a, b])
    assert set(result) == {a, b}


def test_three_points_middle_one_dominated():
    best_quality = ParetoPoint("best_quality", wer=0.30, realtime_factor=5.0)
    dominated = ParetoPoint("dominated", wer=0.35, realtime_factor=5.0)  # same speed, worse WER than best_quality
    best_speed = ParetoPoint("best_speed", wer=0.45, realtime_factor=40.0)
    result = pareto_frontier([best_quality, dominated, best_speed])
    assert set(result) == {best_quality, best_speed}


def test_matches_real_findings_shape():
    # Loosely modeled on real_runs/FINDINGS.md's numbers: Canary is both
    # worse WER and far slower than the Parakeet variants at the same
    # window/overlap -- it should never appear on the frontier here.
    canary = ParetoPoint("canary", wer=0.355, realtime_factor=5.3)
    parakeet_v2 = ParetoPoint("parakeet_v2", wer=0.356, realtime_factor=19.5)
    parakeet_v3 = ParetoPoint("parakeet_v3", wer=0.334, realtime_factor=19.6)
    onnx_int8 = ParetoPoint("onnx_int8", wer=0.348, realtime_factor=38.9)
    result = pareto_frontier([canary, parakeet_v2, parakeet_v3, onnx_int8])
    assert canary not in result
    assert parakeet_v3 in result  # best WER of the four
    assert onnx_int8 in result  # best throughput of the four
