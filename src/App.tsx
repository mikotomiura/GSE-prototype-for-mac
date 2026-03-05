import { useEffect, useState, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import Dashboard from "./components/Dashboard";
import Overlay from "./components/Overlay";
import "./App.css";

interface CognitiveStateRaw {
  flow: number;
  incubation: number;
  stuck: number;
}

// Lv1トリガー閾値: NudgeはOverlay.tsxで stuck>0.6 時に表示
// Lv2トリガー閾値: Wallは stuck>0.7 の累積時間が30秒に達した際に表示
const LV2_STUCK_THRESHOLD = 0.7;
const LV2_SLACK_THRESHOLD = 0.5; // これ未満で累積を減衰（明確な回復）
const LV2_TIMER_MS = 30_000;

function App() {
  const [cognitiveState, setCognitiveState] = useState<CognitiveStateRaw>({
    flow: 0.80,
    incubation: 0.15,
    stuck: 0.05,
  });
  const [isWallActive, setIsWallActive] = useState(false);
  // null = 初回確認中, true = フック有効, false = 権限未付与
  const [hookActive, setHookActive] = useState<boolean | null>(null);
  const [keyboardIdleMs, setKeyboardIdleMs] = useState(0);
  // Monk Mode: true = Wall 自動介入を無効化
  const [isMonkMode, setIsMonkMode] = useState(false);
  // セッション開始状態: false = スタート画面, true = ダッシュボード
  const [isStarted, setIsStarted] = useState(false);

  // D-1: getCurrentWindow() を useMemo でメモ化し不要な再生成を防止
  const currentWindow = useMemo(() => getCurrentWindow(), []);
  const windowLabel = currentWindow.label;

  // 1. フック（Input Monitoring）権限チェック — 起動直後に1回確認
  useEffect(() => {
    invoke<boolean>("get_hook_status")
      .then(setHookActive)
      .catch(() => setHookActive(true)); // エラー時は楽観的に true
  }, []);

  // Lv2 Wall 累積カウンター用 ref（ポーリング useEffect 内で参照）
  const stuckAccMsRef = useRef(0);
  const lastStuckTickRef = useRef(Date.now());
  const isWallActiveRef = useRef(isWallActive);
  const isMonkModeRef = useRef(isMonkMode);
  useEffect(() => { isWallActiveRef.current = isWallActive; }, [isWallActive]);
  useEffect(() => { isMonkModeRef.current = isMonkMode; }, [isMonkMode]);

  // 2. Poll Cognitive State (Every 500ms) + Keyboard Idle + Lv2 Wall累積
  //    オーバーレイは常時ポーリング、メインウィンドウは isStarted 依存
  //    Wall累積ロジックもここで実行（500ms間隔が保証されるため）
  useEffect(() => {
    if (windowLabel !== "overlay" && !isStarted) return;
    lastStuckTickRef.current = Date.now();

    const interval = setInterval(async () => {
      try {
        const [state, idle] = await Promise.all([
          invoke<CognitiveStateRaw>("get_cognitive_state"),
          invoke<number>("get_keyboard_idle_ms"),
        ]);
        setCognitiveState(state);
        setKeyboardIdleMs(idle);

        // Lv2 Wall 累積カウンター（ヒステリシスバンド + 減衰付き）
        const now = Date.now();
        const elapsed = now - lastStuckTickRef.current;
        lastStuckTickRef.current = now;

        if (!isWallActiveRef.current && !isMonkModeRef.current) {
          if (state.stuck > LV2_STUCK_THRESHOLD) {
            stuckAccMsRef.current += elapsed;
            if (stuckAccMsRef.current >= LV2_TIMER_MS) {
              setIsWallActive(true);
              stuckAccMsRef.current = 0;
            }
          } else if (state.stuck < LV2_SLACK_THRESHOLD) {
            // 明確な回復: 2倍速で減衰（瞬時リセットより寛容）
            stuckAccMsRef.current = Math.max(0, stuckAccMsRef.current - elapsed * 2);
          }
          // else: SLACK ≤ stuck ≤ STUCK → 累積一時停止（遊び/スラック）
        } else {
          stuckAccMsRef.current = 0;
        }
      } catch (e) {
        console.error("Failed to fetch state:", e);
      }
    }, 500);

    return () => clearInterval(interval);
  }, [isStarted, windowLabel]);

  // 4. Sensor Integration (Unlock Logic) + メインウィンドウへのフォーカス復帰
  useEffect(() => {
    const unlisten = listen("sensor-accelerometer", (event) => {
      if (event.payload === "move") {
        setIsWallActive(false);
        // メインウィンドウへフォーカスを自動復帰
        if (windowLabel !== "overlay") {
          currentWindow.setFocus().catch(() => {});
        }
      }
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, [windowLabel, currentWindow]);

  // 5. Monk Mode — 両ウィンドウ間で同期するため Tauri イベントを使用
  useEffect(() => {
    const unlisten = listen<boolean>("monk-mode-changed", (event) => {
      setIsMonkMode(event.payload);
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // Render based on Window Label
  if (windowLabel === "overlay") {
    return (
      <Overlay
        stuckProb={cognitiveState.stuck}
        isWallActive={isWallActive}
      />
    );
  }

  const handleStart = async () => {
    try {
      await invoke("start_session");
      setIsStarted(true);
    } catch (e) {
      console.error("Failed to start session:", e);
    }
  };

  const handleQuit = () => {
    invoke("quit_app").catch(console.error);
  };

  // スタート画面: セッション未開始時
  if (!isStarted) {
    return (
      <div className="start-screen">
        <div className="start-header">
          <h1>Generative Struggle Engine</h1>
          <p className="start-description">
            キーストロークを解析し、あなたの認知状態をリアルタイムで推定します。
          </p>
        </div>
        <button className="start-button" onClick={handleStart}>
          開始する
        </button>
        <p className="start-hint">
          自然にタイピングを始めると、検知が動作します。
        </p>
      </div>
    );
  }

  // Default to Main Dashboard
  return (
    <Dashboard
      cognitiveState={cognitiveState}
      onQuit={handleQuit}
      hookActive={hookActive}
      keyboardIdleMs={keyboardIdleMs}
      isMonkMode={isMonkMode}
    />
  );
}

export default App;
