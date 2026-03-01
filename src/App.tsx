import { useEffect, useState, useMemo } from "react";
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
// Lv2トリガー閾値: Wallは stuck>0.7 が 30秒継続した際に表示
const LV2_STUCK_THRESHOLD = 0.7;
const LV2_TIMER_MS = 30_000;

function App() {
  const [cognitiveState, setCognitiveState] = useState<CognitiveStateRaw>({
    flow: 0.5,
    incubation: 0.3,
    stuck: 0.2,
  });
  const [isWallActive, setIsWallActive] = useState(false);
  // null = 初回確認中, true = フック有効, false = 権限未付与
  const [hookActive, setHookActive] = useState<boolean | null>(null);

  // D-1: getCurrentWindow() を useMemo でメモ化し不要な再生成を防止
  const currentWindow = useMemo(() => getCurrentWindow(), []);
  const windowLabel = currentWindow.label;

  // 1. フック（Input Monitoring）権限チェック — 起動直後に1回確認
  useEffect(() => {
    invoke<boolean>("get_hook_status")
      .then(setHookActive)
      .catch(() => setHookActive(true)); // エラー時は楽観的に true
  }, []);

  // 2. Poll Cognitive State (Every 500ms)
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const state = await invoke<CognitiveStateRaw>("get_cognitive_state");
        setCognitiveState(state);
      } catch (e) {
        console.error("Failed to fetch state:", e);
      }
    }, 500);

    return () => clearInterval(interval);
  }, []);

  // 3. Intervention Layer ロジック
  //    D-2: Lv1 (Nudge) は Overlay.tsx 側で stuck>0.6 時に即時表示済み
  //    D-3: Lv2 (Ambient Fade/Wall) は stuck>LV2_STUCK_THRESHOLD が LV2_TIMER_MS 継続で発動
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    if (cognitiveState.stuck > LV2_STUCK_THRESHOLD && !isWallActive) {
      // Stuck状態が閾値を超えたまま 30秒継続したら Wall (Lv2) を発動
      timer = setTimeout(() => {
        setIsWallActive(true);
      }, LV2_TIMER_MS);
    }
    return () => clearTimeout(timer);
  }, [cognitiveState.stuck, isWallActive]);

  // 4. Sensor Integration (Unlock Logic)
  useEffect(() => {
    const unlisten = listen("sensor-accelerometer", (event) => {
      if (event.payload === "move") {
        setIsWallActive(false);
      }
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

  const handleQuit = () => {
    invoke("quit_app").catch(console.error);
  };

  // Default to Main Dashboard
  return (
    <Dashboard
      cognitiveState={cognitiveState}
      onQuit={handleQuit}
      hookActive={hookActive}
    />
  );
}

export default App;
