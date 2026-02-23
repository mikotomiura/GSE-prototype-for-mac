"""
behavioral_gt.py — 行動シーケンスによる客観的 GT ラベリング
=============================================================
advice.md「修正案A: 行動シーケンスによる完全客観ラベリング」の実装。

入力: logger.rs が生成した NDJSON ファイル (gse_YYYYMMDD_HHMMSS.ndjson)
出力: session_YYYYMMDD_labeled.csv

使用例:
    python behavioral_gt.py gse_20260221_143022.ndjson
    python behavioral_gt.py gse_20260221_143022.ndjson --window 30 --step 1

ラベル定義 (advice.md 準拠):
    FLOW        : median(FT) < 200ms かつ correction_rate < 0.15 かつ STUCK/INC でない
    INCUBATION  : Pause(≥2000ms) → 30秒以内に Burst(5+文字, FT<200ms) → diff_chars ≥3
    STUCK       : 「Burst(≤3) → Delete → Pause(≥2s)」ループ ≥3回 in 60s、diff_chars ≤0
    UNKNOWN     : 複数ラベルが重複する区間、または条件を満たさない区間
"""

from __future__ import annotations

import argparse
import csv
import json
import statistics
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

# ---------------------------------------------------------------------------
# データ型
# ---------------------------------------------------------------------------

Label = Literal["FLOW", "INCUBATION", "STUCK", "UNKNOWN"]

VK_BACK   = 0x08
VK_DELETE = 0x2E
CHAR_KEYS = set(range(0x30, 0x5B)) | set(range(0x60, 0x70))  # 0-9, A-Z, numpad


@dataclass
class KeyEvent:
    t: int        # timestamp (ms)
    vk: int       # virtual key code
    press: bool   # True=keydown, False=keyup


@dataclass
class FeatEvent:
    t: int
    f1: float     # flight time median (ms)
    f2: float     # flight time variance
    f3: float     # correction rate
    f4: float     # burst length
    f5: float     # pause count
    f6: float     # pause-after-delete rate
    p_flow: float
    p_inc: float
    p_stuck: float


@dataclass
class LabeledSegment:
    start_ms: int
    end_ms: int
    label: Label
    evidence: str = ""  # デバッグ用メモ


# ---------------------------------------------------------------------------
# NDJSON 読み込み
# ---------------------------------------------------------------------------

def load_ndjson(path: Path) -> tuple[list[KeyEvent], list[FeatEvent]]:
    keys: list[KeyEvent] = []
    feats: list[FeatEvent] = []

    with open(path, encoding="utf-8") as f:
        for line_no, raw in enumerate(f, 1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError as e:
                print(f"[warn] line {line_no}: JSON parse error: {e}", file=sys.stderr)
                continue

            t = obj.get("type")
            if t == "key":
                keys.append(KeyEvent(
                    t=obj["t"],
                    vk=obj["vk"],
                    press=obj["press"],
                ))
            elif t == "feat":
                feats.append(FeatEvent(
                    t=obj["t"],
                    f1=obj.get("f1", 0.0),
                    f2=obj.get("f2", 0.0),
                    f3=obj.get("f3", 0.0),
                    f4=obj.get("f4", 0.0),
                    f5=obj.get("f5", 0.0),
                    f6=obj.get("f6", 0.0),
                    p_flow=obj.get("p_flow", 0.0),
                    p_inc=obj.get("p_inc", 0.0),
                    p_stuck=obj.get("p_stuck", 0.0),
                ))

    return keys, feats


# ---------------------------------------------------------------------------
# ユーティリティ
# ---------------------------------------------------------------------------

def is_char_key(vk: int) -> bool:
    """文字系キー (英数字・テンキー) かどうか"""
    return vk in CHAR_KEYS or (0xBA <= vk <= 0xE2)  # OEM keys


def is_delete_key(vk: int) -> bool:
    return vk == VK_BACK or vk == VK_DELETE


def flight_times_in_window(
    keys: list[KeyEvent], start_ms: int, end_ms: int
) -> list[float]:
    """指定ウィンドウ内のフライトタイム (release→next press) を返す"""
    window = [k for k in keys if start_ms <= k.t < end_ms]
    fts: list[float] = []
    last_release: int | None = None
    for k in window:
        if not k.press:
            last_release = k.t
        elif last_release is not None:
            ft = k.t - last_release
            if 0 < ft < 2000:
                fts.append(float(ft))
    return fts


def diff_chars_in_window(
    keys: list[KeyEvent], start_ms: int, end_ms: int
) -> int:
    """指定ウィンドウ内の文字純増数 (文字キーdown - 削除キーdown) を返す"""
    presses = [k for k in keys if start_ms <= k.t < end_ms and k.press]
    chars_added   = sum(1 for k in presses if is_char_key(k.vk))
    chars_deleted = sum(1 for k in presses if is_delete_key(k.vk))
    return chars_added - chars_deleted


# ---------------------------------------------------------------------------
# ラベリング関数
# ---------------------------------------------------------------------------

def label_stuck(
    keys: list[KeyEvent], win_start: int, win_end: int
) -> bool:
    """
    60秒ウィンドウ内に以下パターンが ≥3回繰り返される:
      Burst(≤3文字) → Delete(≥1) → Pause(≥2000ms)
    かつ diff_chars ≤ 0

    根拠: F6（Pause-after-Delete率）の操作的定義の強化版。
    「書く→消す→固まる」という彷徨ループを捕捉。
    """
    if diff_chars_in_window(keys, win_start, win_end) > 0:
        return False

    presses = [k for k in keys if win_start <= k.t < win_end and k.press]
    if len(presses) < 5:
        return False

    # パターン検出: Burst → Delete → Pause
    loop_count = 0
    i = 0
    while i < len(presses):
        # Burst: 連続する文字キー ≤3文字
        burst_start = i
        burst_chars = 0
        while i < len(presses) and is_char_key(presses[i].vk):
            burst_chars += 1
            i += 1
        if burst_chars == 0 or burst_chars > 3:
            i = max(i, burst_start + 1)
            continue

        # Delete: ≥1回の削除
        delete_count = 0
        while i < len(presses) and is_delete_key(presses[i].vk):
            delete_count += 1
            i += 1
        if delete_count == 0:
            continue

        # Pause: 次のキーまでの間隔 ≥2000ms
        if i < len(presses):
            gap = presses[i].t - presses[i - 1].t
            if gap >= 2000:
                loop_count += 1
        elif i == len(presses):
            # ウィンドウ末尾で終わった場合もPauseとみなす
            gap = win_end - presses[-1].t
            if gap >= 2000:
                loop_count += 1

        if loop_count >= 3:
            return True

    return loop_count >= 3


def label_incubation(
    keys: list[KeyEvent], win_start: int, win_end: int
) -> bool:
    """
    ウィンドウ内に以下のシーケンスが完結する:
      1. Pause (無入力 ≥2000ms)
      2. Pause終了から30秒以内に Burst (連続5文字以上、各FT < 200ms)
      3. そのBurst後30秒間の diff_chars ≥ 3

    根拠: RH2「長いPause → 高速連続Burst」がIncubationの特徴。
    """
    presses = [k for k in keys if win_start <= k.t < win_end and k.press]
    if len(presses) < 7:
        return False

    for i in range(1, len(presses)):
        gap = presses[i].t - presses[i - 1].t

        if gap < 2000:
            continue

        # Pause 検出: gap ≥ 2000ms
        pause_end_t = presses[i].t

        # Burst: pause_end から 30秒以内に 5文字以上を FT<200ms で連続入力
        burst_start_idx = i
        burst_count = 0
        burst_end_t: int | None = None

        for j in range(burst_start_idx, len(presses)):
            if presses[j].t > pause_end_t + 30_000:
                break
            if is_char_key(presses[j].vk):
                if j == burst_start_idx:
                    burst_count = 1
                    burst_end_t = presses[j].t
                else:
                    ft = presses[j].t - presses[j - 1].t
                    if ft < 200:
                        burst_count += 1
                        burst_end_t = presses[j].t
                    else:
                        if burst_count >= 5:
                            break
                        burst_count = 1
                        burst_end_t = presses[j].t

        if burst_count < 5 or burst_end_t is None:
            continue

        # diff_chars: Burst後30秒
        dc = diff_chars_in_window(keys, burst_end_t, burst_end_t + 30_000)
        if dc >= 3:
            return True

    return False


def label_flow(
    keys: list[KeyEvent], win_start: int, win_end: int,
    is_stuck: bool, is_inc: bool
) -> bool:
    """
    median(FT) < 200ms かつ correction_rate < 0.15
    かつ STUCK でも INCUBATION でもない

    根拠: 高速・低修正率を Flow の操作的定義とする。
    """
    if is_stuck or is_inc:
        return False

    presses = [k for k in keys if win_start <= k.t < win_end and k.press]
    if len(presses) < 5:
        return False

    fts = flight_times_in_window(keys, win_start, win_end)
    if not fts:
        return False

    med_ft = statistics.median(fts)
    if med_ft >= 200.0:
        return False

    total = len(presses)
    corrections = sum(1 for k in presses if is_delete_key(k.vk))
    corr_rate = corrections / total if total > 0 else 0.0

    return corr_rate < 0.15


# ---------------------------------------------------------------------------
# スライディングウィンドウでラベル付与
# ---------------------------------------------------------------------------

def label_session(
    keys: list[KeyEvent],
    window_ms: int = 30_000,
    step_ms: int = 1_000,
) -> list[LabeledSegment]:
    """
    1秒ステップ・30秒幅のスライディングウィンドウでラベルを生成する。
    """
    if not keys:
        return []

    segments: list[LabeledSegment] = []
    t_start = keys[0].t
    t_end   = keys[-1].t + step_ms

    t = t_start
    while t < t_end:
        win_start = t
        win_end   = t + window_ms

        stuck = label_stuck(keys, win_start, win_end)
        inc   = label_incubation(keys, win_start, win_end)
        flow  = label_flow(keys, win_start, win_end, stuck, inc)

        n_labels = sum([stuck, inc, flow])
        if n_labels > 1:
            lbl: Label = "UNKNOWN"
            ev = "multi-label conflict"
        elif stuck:
            lbl = "STUCK"
            ev = "burst<=3 -> delete -> pause >=3x, diff_chars<=0"
        elif inc:
            lbl = "INCUBATION"
            ev = "pause -> burst(5+) -> diff_chars>=3"
        elif flow:
            lbl = "FLOW"
            ev = "median_ft<200ms, correction_rate<0.15"
        else:
            lbl = "UNKNOWN"
            ev = "no condition met"

        segments.append(LabeledSegment(
            start_ms=win_start,
            end_ms=win_end,
            label=lbl,
            evidence=ev,
        ))

        t += step_ms

    return segments


# ---------------------------------------------------------------------------
# CSV 出力
# ---------------------------------------------------------------------------

def write_csv(segments: list[LabeledSegment], out_path: Path) -> None:
    with open(out_path, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["start_ms", "end_ms", "label", "evidence"])
        w.writeheader()
        for s in segments:
            w.writerow({
                "start_ms": s.start_ms,
                "end_ms":   s.end_ms,
                "label":    s.label,
                "evidence": s.evidence,
            })
    print(f"Wrote {len(segments)} segments to {out_path}")


def print_summary(segments: list[LabeledSegment]) -> None:
    from collections import Counter
    counts = Counter(s.label for s in segments)
    total = len(segments)
    print("\n=== Label Summary ===")
    for lbl in ("FLOW", "INCUBATION", "STUCK", "UNKNOWN"):
        n = counts.get(lbl, 0)
        pct = 100 * n / total if total else 0
        print(f"  {lbl:<12}: {n:>5} windows  ({pct:5.1f}%)")
    print(f"  {'TOTAL':<12}: {total:>5} windows")
    print()


# ---------------------------------------------------------------------------
# エントリポイント
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Behavioral GT labeling for GSE session NDJSON files"
    )
    parser.add_argument("ndjson", type=Path, help="Input NDJSON session file")
    parser.add_argument(
        "--window", type=int, default=30,
        help="Sliding window size in seconds (default: 30)"
    )
    parser.add_argument(
        "--step", type=int, default=1,
        help="Step size in seconds (default: 1)"
    )
    parser.add_argument(
        "--out", type=Path, default=None,
        help="Output CSV path (default: <input>_labeled.csv)"
    )
    args = parser.parse_args()

    if not args.ndjson.exists():
        print(f"Error: file not found: {args.ndjson}", file=sys.stderr)
        sys.exit(1)

    print(f"Loading {args.ndjson} ...")
    keys, feats = load_ndjson(args.ndjson)
    print(f"  Loaded {len(keys)} key events, {len(feats)} feature snapshots")

    if not keys:
        print("No key events found. Exiting.", file=sys.stderr)
        sys.exit(1)

    print(f"Labeling (window={args.window}s, step={args.step}s) ...")
    segments = label_session(
        keys,
        window_ms=args.window * 1000,
        step_ms=args.step * 1000,
    )

    print_summary(segments)

    out_path = args.out or args.ndjson.with_suffix("").with_name(
        args.ndjson.stem + "_labeled.csv"
    )
    write_csv(segments, out_path)


if __name__ == "__main__":
    main()
