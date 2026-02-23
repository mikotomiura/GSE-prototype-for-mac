README.md ファイルを修正してください。
先日受けたメンターからの学術的なアドバイス（HSMMの理論的妥当性、順序依存確率による状態定義、意味的潜在軸の導入）を反映し、システムの理論的基盤を強化することが目的です。

以下の3つの変更を適用してください。

### 変更1: `## Cognitive State Model` セクションの表の更新
「Behavioral Signature」の列を、順序依存確率（条件付き確率）を用いた学術的な定義に書き換えてください。

【変更前の該当部分】
| **Incubation** | Deliberate pause enabling sub-conscious problem restructuring (Sio & Ormerod, 2009) | Extended silence (≥2 s) followed by rapid output burst |
| **Stuck** | Perseverative failure to escape an impasse (Ohlsson, 1992) | Short bursts → delete → pause loops with zero net character gain |

【変更後の該当部分】
| **Incubation** | Deliberate pause enabling sub-conscious problem restructuring (Sio & Ormerod, 2009) | **High $P(\text{Burst} \mid \text{Pause})$**: Extended silence (≥2 s) followed by rapid output burst |
| **Stuck** | Perseverative failure to escape an impasse (Ohlsson, 1992) | **High $P(\text{Pause} \mid \text{Delete})$**: Perseverative delete-pause loops with near-zero character gain |


### 変更2: `### Latent Axes: Friction × Engagement` セクションの名称と数式の更新
単なるヒューリスティックな軸ではなく、「意味的潜在軸」であることを宣言し、Engagementを「Productive Silence」に変更してください。また、それに伴いObservation Binsの軸名も変更してください。

【指示内容】
1. 見出しを `### Semantic Latent Axes: Cognitive Friction × Productive Silence` に変更する。
2. 軸の数式定義部分を以下のように書き換える。

```markdown
The six normalized features are projected onto two interpretable semantic latent axes before discretization:

```text
X (Cognitive Friction)   = 0.30·φ(F3) + 0.25·φ(F6) + 0.25·φ(F1) + 0.20·φ(F5)
Y (Productive Silence)   = 0.40·φ(F4) + 0.35·(1 − φ(F1)) + 0.25·(1 − φ(F5))

Cognitive Friction ($X$): Quantifies the depth of "hesitation" or struggle, heavily weighting the Stuck index $P(\text{Pause} \mid \text{Delete})$ (represented by F6).Productive Silence ($Y$): Indicates how much a silence leads to a productive burst. This separates valuable DMN-activated incubation from mere cognitive stalling.
3. 直下にある `### Observation Bins` の図表の軸ラベルを以下のように書き換える。
`Friction X →` を `Cognitive Friction X →` に変更。
`Engagement Y ↓` を `Productive Silence Y ↓` に変更。


### 変更3: `### Fix ①: Cold-Start Hysteresis` セクションへのHSMM理論的背景の追加
「HMMは持続時間を持てない」という理論的弱点に対し、なぜHSMM（Hidden Semi-Markov Model）ではなくEMAヒステリシスを導入したのかという設計の妥当性を追加してください。

【指示内容】
見出し `### Fix ①: Cold-Start Hysteresis (Stuck → Flow window-reset spike)` の直下（**Problem:** の前）に、以下の理論的背景（Theoretical Note）を挿入してください。

```markdown
**Theoretical Note: O(1) Alternative to HSMM**
Standard HMMs cannot model state duration distributions explicitly (they assume geometric decay). While a Hidden Semi-Markov Model (HSMM) is theoretically optimal for modeling the distinct, non-geometric durations of Incubation and Stuck, it introduces $O(T^2)$ computational complexity and requires massive data to estimate duration parameters (overfitting in $n=1$ environments). The EMA hysteresis layer introduced below acts as an $O(1)$ computational hack to enforce minimum state dwell times without the overhead of an HSMM, making it ideal for edge inference.