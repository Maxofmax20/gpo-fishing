"""Classification metrics + calibration (pure functions, fully tested logic).

All metrics derive from (true, pred) label lists and optional probability
matrices. No data loading here.
"""

from __future__ import annotations

import math


def confusion(y_true: list[int], y_pred: list[int], n_classes: int) -> list[list[int]]:
    m = [[0] * n_classes for _ in range(n_classes)]
    for t, p in zip(y_true, y_pred):
        if 0 <= t < n_classes and 0 <= p < n_classes:
            m[t][p] += 1
    return m


def accuracy(y_true: list[int], y_pred: list[int]) -> float:
    if not y_true:
        return 0.0
    return sum(1 for t, p in zip(y_true, y_pred) if t == p) / len(y_true)


def per_class_prf(y_true: list[int], y_pred: list[int], n_classes: int) -> list[dict]:
    m = confusion(y_true, y_pred, n_classes)
    out = []
    for c in range(n_classes):
        tp = m[c][c]
        fp = sum(m[r][c] for r in range(n_classes)) - tp
        fn = sum(m[c]) - tp
        prec = tp / (tp + fp) if (tp + fp) else 0.0
        rec = tp / (tp + fn) if (tp + fn) else 0.0
        f1 = 2 * prec * rec / (prec + rec) if (prec + rec) else 0.0
        out.append({"precision": prec, "recall": rec, "f1": f1, "support": tp + fn})
    return out


def macro_f1(y_true: list[int], y_pred: list[int], n_classes: int) -> float:
    if not y_true:
        return 0.0
    prf = per_class_prf(y_true, y_pred, n_classes)
    return sum(c["f1"] for c in prf) / n_classes


def balanced_accuracy(y_true: list[int], y_pred: list[int], n_classes: int) -> float:
    if not y_true:
        return 0.0
    prf = per_class_prf(y_true, y_pred, n_classes)
    return sum(c["recall"] for c in prf) / n_classes


def log_loss(probs: list[list[float]], y_true: list[int]) -> float:
    if not y_true:
        return 0.0
    tot = 0.0
    for p, t in zip(probs, y_true):
        tot += -math.log(max(1e-12, min(1.0 - 1e-12, p[t])))
    return tot / len(y_true)


def brier_score(probs: list[list[float]], y_true: list[int]) -> float:
    if not y_true:
        return 0.0
    tot = 0.0
    for p, t in zip(probs, y_true):
        tot += sum(((1.0 if c == t else 0.0) - pi) ** 2 for c, pi in enumerate(p))
    return tot / len(y_true)


def expected_calibration_error(probs: list[list[float]], y_true: list[int], bins: int = 10) -> float:
    """ECE over max-probability confidence bins."""
    if not y_true:
        return 0.0
    sums = [0.0] * bins
    hits = [0] * bins
    counts = [0] * bins
    for p, t in zip(probs, y_true):
        conf = max(p)
        pred = p.index(conf)
        b = min(bins - 1, int(conf * bins))
        sums[b] += conf
        counts[b] += 1
        if pred == t:
            hits[b] += 1
    ece = 0.0
    n = len(y_true)
    for b in range(bins):
        if counts[b]:
            ece += abs(sums[b] / counts[b] - hits[b] / counts[b]) * (counts[b] / n)
    return ece


def apply_temperature(logits: list[list[float]], temperature: float) -> list[list[float]]:
    out = []
    for row in logits:
        m = max(row)
        exps = [math.exp((v - m) / max(1e-6, temperature)) for v in row]
        s = sum(exps)
        out.append([e / s for e in exps])
    return out


def fit_temperature(val_logits: list[list[float]], val_true: list[int]) -> dict:
    """Grid-search temperature on VALIDATION data only (never test)."""
    best = {"temperature": 1.0, "log_loss": log_loss(apply_temperature(val_logits, 1.0), val_true)}
    t = 0.25
    while t <= 4.0:
        ll = log_loss(apply_temperature(val_logits, t), val_true)
        if ll < best["log_loss"]:
            best = {"temperature": round(t, 4), "log_loss": ll}
        t += 0.25
    cands = [best["temperature"] - 0.2, best["temperature"] - 0.1, best["temperature"] + 0.1, best["temperature"] + 0.2]
    for t in cands:
        if t < 0.1:
            continue
        ll = log_loss(apply_temperature(val_logits, t), val_true)
        if ll < best["log_loss"]:
            best = {"temperature": round(t, 4), "log_loss": ll}
    return best
