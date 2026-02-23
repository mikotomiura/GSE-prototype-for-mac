"""
hmm_sensitivity.py — HMM 遷移確率の感度分析
==============================================
advice.md「センシティビティ分析」の実装。

HMM遷移確率パラメータを変化させたとき、
Stuck検出遅延（P(stuck) > 0.7 に達するまでの秒数）が
±2秒以内に収まることを確認し、ヒートマップで可視化する。

使用例:
    python hmm_sensitivity.py
    python hmm_sensitivity.py --sims 500 --out sensitivity_heatmap.png

依存: numpy, matplotlib
    pip install numpy matplotlib
"""

from __future__ import annotations

import argparse
import itertools
import sys
from typing import NamedTuple

import numpy as np

try:
    import matplotlib.pyplot as plt
    import matplotlib.colors as mcolors
    HAS_MATPLOTLIB = True
except ImportError:
    HAS_MATPLOTLIB = False
    print("[warn] matplotlib not found. Install with: pip install matplotlib", file=sys.stderr)


# ---------------------------------------------------------------------------
# HMM 定義
# ---------------------------------------------------------------------------

N_STATES = 3  # Flow=0, Incubation=1, Stuck=2
N_OBS    = 11  # S_stuck を 11 ビンに離散化


def build_emissions() -> np.ndarray:
    """
    現在の emissions 行列 (engine.rs と同じ値)。
    感度分析では emissions は固定し、transitions のみ変化させる。
    """
    return np.array([
        # Flow: 低ビン集中
        [0.35, 0.25, 0.15, 0.10, 0.07, 0.05, 0.02, 0.01, 0.0,  0.0,  0.0 ],
        # Incubation: 中〜高ビン
        [0.02, 0.03, 0.05, 0.08, 0.10, 0.15, 0.20, 0.20, 0.10, 0.06, 0.01],
        # Stuck: 高ビンのみ
        [0.0,  0.0,  0.01, 0.02, 0.05, 0.10, 0.20, 0.30, 0.20, 0.10, 0.99],
    ], dtype=float)


def build_transitions(
    flow_to_stuck: float,
    stuck_to_flow: float,
    stuck_to_stuck: float,
) -> np.ndarray:
    """
    遷移行列を構築する。
    各行の合計が 1.0 になるよう調整する。

    固定パラメータ:
      Flow → Flow:             0.92  (残りを flow_to_stuck で割り当て)
      Flow → Incubation:       1.0 - 0.92 - flow_to_stuck
      Incubation → Flow:       0.10
      Incubation → Incubation: 0.82
      Incubation → Stuck:      0.08
      Stuck → Flow:            stuck_to_flow
      Stuck → Incubation:      1.0 - stuck_to_stuck - stuck_to_flow
      Stuck → Stuck:           stuck_to_stuck
    """
    flow_to_inc   = 1.0 - 0.92 - flow_to_stuck
    stuck_to_inc  = 1.0 - stuck_to_stuck - stuck_to_flow

    # 負になるパラメータはスキップ（無効な組み合わせ）
    if flow_to_inc < 0 or stuck_to_inc < 0:
        return None  # type: ignore

    return np.array([
        [0.92,         flow_to_inc,  flow_to_stuck ],
        [0.10,         0.82,         0.08          ],
        [stuck_to_flow, stuck_to_inc, stuck_to_stuck],
    ], dtype=float)


# ---------------------------------------------------------------------------
# 合成 Stuck シナリオ生成
# ---------------------------------------------------------------------------

def generate_stuck_observations(n_steps: int, rng: np.random.Generator) -> np.ndarray:
    """
    Stuck 状態の観測ビン列を合成生成する。

    Stuck ユーザーの典型的なS_stuckスコア:
      - FT=800ms, correction_rate=0.25, burst_length=1, pause_count=6
      → S_stuck ≈ 0.7 〜 0.9 (ビン 7〜9)
    ここでは bin=7 を中心とした分布を使う（若干のノイズ付き）。
    """
    center = 7.0
    std    = 1.0
    raw = rng.normal(center, std, n_steps)
    bins = np.clip(np.round(raw).astype(int), 0, 10)
    return bins


# ---------------------------------------------------------------------------
# HMM フォワードアルゴリズム (単ステップ)
# ---------------------------------------------------------------------------

def hmm_forward_step(
    probs: np.ndarray,           # shape (3,)
    obs: int,                    # 観測ビン 0-10
    transitions: np.ndarray,     # shape (3, 3)
    emissions: np.ndarray,       # shape (3, 11)
) -> np.ndarray:
    """1ステップのフォワード更新 (正規化済み)"""
    # 遷移後の事前確率
    prior = transitions.T @ probs            # shape (3,)
    # 観測による尤度
    likelihood = prior * emissions[:, obs]   # shape (3,)
    s = likelihood.sum()
    if s > 0:
        return likelihood / s
    return probs  # 分母ゼロ時は前の状態を保持


# ---------------------------------------------------------------------------
# 1シミュレーション: 検出遅延を計算
# ---------------------------------------------------------------------------

STUCK_THRESHOLD = 0.7   # P(stuck) がこの値を超えたら「検出」
INITIAL_PROBS   = np.array([0.7, 0.2, 0.1], dtype=float)


class SimResult(NamedTuple):
    detected: bool     # 閾値を超えたか
    delay_steps: int   # 検出までのステップ数 (秒)


def run_simulation(
    transitions: np.ndarray,
    emissions: np.ndarray,
    n_steps: int,
    rng: np.random.Generator,
) -> SimResult:
    """Stuck シナリオを n_steps 秒間シミュレートし、検出遅延を返す"""
    obs_seq = generate_stuck_observations(n_steps, rng)
    probs = INITIAL_PROBS.copy()

    for step, obs in enumerate(obs_seq):
        probs = hmm_forward_step(probs, obs, transitions, emissions)
        if probs[2] >= STUCK_THRESHOLD:  # Stuck = index 2
            return SimResult(detected=True, delay_steps=step + 1)

    return SimResult(detected=False, delay_steps=n_steps)


# ---------------------------------------------------------------------------
# 感度分析メイン
# ---------------------------------------------------------------------------

PARAM_RANGES = {
    "flow_to_stuck":  [0.001, 0.005, 0.01, 0.02, 0.05],
    "stuck_to_flow":  [0.01,  0.02,  0.05, 0.10],
    "stuck_to_stuck": [0.80,  0.83,  0.85, 0.88, 0.90],
}


def run_sensitivity_analysis(
    n_simulations: int = 1000,
    n_steps: int = 30,
    seed: int = 42,
) -> dict:
    """
    全パラメータ組み合わせで n_simulations 回シミュレーションを行い、
    平均検出遅延 (秒) を記録する。

    Returns:
        results[stuck_to_stuck][stuck_to_flow] = mean_delay_seconds
    """
    emissions = build_emissions()
    rng = np.random.default_rng(seed)

    results: dict = {}  # (s2s, s2f) -> mean_delay

    total_combos = (
        len(PARAM_RANGES["flow_to_stuck"])
        * len(PARAM_RANGES["stuck_to_flow"])
        * len(PARAM_RANGES["stuck_to_stuck"])
    )
    print(f"Running {n_simulations} simulations × {total_combos} parameter combinations...")

    for f2s, s2f, s2s in itertools.product(
        PARAM_RANGES["flow_to_stuck"],
        PARAM_RANGES["stuck_to_flow"],
        PARAM_RANGES["stuck_to_stuck"],
    ):
        T = build_transitions(
            flow_to_stuck=f2s,
            stuck_to_flow=s2f,
            stuck_to_stuck=s2s,
        )
        if T is None:
            continue  # 無効な組み合わせをスキップ

        delays = []
        for _ in range(n_simulations):
            res = run_simulation(T, emissions, n_steps, rng)
            delays.append(res.delay_steps)

        mean_delay = float(np.mean(delays))
        results[(s2s, s2f, f2s)] = mean_delay

    return results


def print_table(results: dict) -> None:
    """テキスト形式で結果を出力"""
    print("\n=== Sensitivity Analysis Results ===")
    print(f"{'stuck→stuck':>12} {'stuck→flow':>12} {'flow→stuck':>12} {'mean_delay(s)':>15}")
    print("-" * 56)

    baseline = results.get(
        (0.80, 0.05, 0.01), None
    )

    for (s2s, s2f, f2s), delay in sorted(results.items()):
        marker = " ← baseline" if (s2s == 0.80 and s2f == 0.05 and f2s == 0.01) else ""
        print(f"{s2s:>12.2f} {s2f:>12.3f} {f2s:>12.3f} {delay:>15.2f}{marker}")

    if baseline is not None:
        delays = list(results.values())
        min_d = min(delays)
        max_d = max(delays)
        print(f"\nBaseline delay : {baseline:.2f}s")
        print(f"Range          : {min_d:.2f}s ~ {max_d:.2f}s")
        print(f"Max deviation  : ±{(max_d - min_d) / 2:.2f}s")

        goal_met = (max_d - min_d) <= 4.0  # ±2秒以内 = 全体幅4秒以内
        print(f"Goal (±2s)    : {'✓ MET' if goal_met else '✗ NOT MET'}")


def plot_heatmap(results: dict, out_path: str) -> None:
    """stuck_to_flow × stuck_to_stuck ヒートマップを生成 (flow_to_stuck=0.01 で固定)"""
    if not HAS_MATPLOTLIB:
        print("[warn] Skipping heatmap (matplotlib not available)")
        return

    f2s_fixed = 0.01  # 代表値で固定
    s2s_vals = sorted(set(k[0] for k in results))
    s2f_vals = sorted(set(k[1] for k in results))

    grid = np.full((len(s2s_vals), len(s2f_vals)), np.nan)
    for i, s2s in enumerate(s2s_vals):
        for j, s2f in enumerate(s2f_vals):
            key = (s2s, s2f, f2s_fixed)
            if key in results:
                grid[i, j] = results[key]

    fig, ax = plt.subplots(figsize=(8, 6))
    im = ax.imshow(
        grid, aspect="auto", origin="lower",
        cmap="RdYlGn_r",
        vmin=0, vmax=30,
    )

    ax.set_xticks(range(len(s2f_vals)))
    ax.set_xticklabels([f"{v:.2f}" for v in s2f_vals])
    ax.set_yticks(range(len(s2s_vals)))
    ax.set_yticklabels([f"{v:.2f}" for v in s2s_vals])
    ax.set_xlabel("stuck→flow probability")
    ax.set_ylabel("stuck→stuck probability")
    ax.set_title(
        f"STUCK Detection Delay (seconds)\n"
        f"[flow→stuck={f2s_fixed}, threshold=P(stuck)>{STUCK_THRESHOLD}]"
    )

    # セルに数値を表示
    for i in range(len(s2s_vals)):
        for j in range(len(s2f_vals)):
            v = grid[i, j]
            if not np.isnan(v):
                color = "white" if v > 20 else "black"
                ax.text(j, i, f"{v:.1f}", ha="center", va="center",
                        fontsize=9, color=color)

    plt.colorbar(im, ax=ax, label="Mean detection delay (s)")

    # 現在のパラメータ（baseline）をマーク
    try:
        xi = s2f_vals.index(0.05)
        yi = s2s_vals.index(0.80)
        ax.add_patch(plt.Rectangle(
            (xi - 0.5, yi - 0.5), 1, 1,
            fill=False, edgecolor="blue", linewidth=2.5, label="current params"
        ))
        ax.legend(loc="upper right")
    except ValueError:
        pass

    plt.tight_layout()
    plt.savefig(out_path, dpi=150)
    print(f"Heatmap saved to {out_path}")
    plt.close()


# ---------------------------------------------------------------------------
# エントリポイント
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="HMM transition probability sensitivity analysis for GSE"
    )
    parser.add_argument(
        "--sims", type=int, default=1000,
        help="Number of simulations per parameter set (default: 1000)"
    )
    parser.add_argument(
        "--steps", type=int, default=30,
        help="Simulation length in steps/seconds (default: 30)"
    )
    parser.add_argument(
        "--out", type=str, default="sensitivity_heatmap.png",
        help="Output heatmap image path (default: sensitivity_heatmap.png)"
    )
    parser.add_argument(
        "--seed", type=int, default=42,
        help="Random seed for reproducibility (default: 42)"
    )
    args = parser.parse_args()

    results = run_sensitivity_analysis(
        n_simulations=args.sims,
        n_steps=args.steps,
        seed=args.seed,
    )

    print_table(results)
    plot_heatmap(results, args.out)


if __name__ == "__main__":
    main()
