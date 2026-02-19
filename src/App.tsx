import { useEffect, useState } from "react";
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

function App() {
  const [windowLabel, setWindowLabel] = useState<string>("");
  const [cognitiveState, setCognitiveState] = useState<CognitiveStateRaw>({
    flow: 0.1,
    incubation: 0.1,
    stuck: 0.0, // Default to neutral/low stuck
  });
  const [isWallActive, setIsWallActive] = useState(false);

  // 1. Identify Window Label
  useEffect(() => {
    // Current window label
    const label = getCurrentWindow().label;
    setWindowLabel(label);
    console.log("Window Label:", label);
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

  // 3. Wall Logic (Stuck Persistence)
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    if (cognitiveState.stuck > 0.9 && !isWallActive) {
      // If Stuck stays high for 3 seconds, activate wall
      timer = setTimeout(() => {
        setIsWallActive(true);
      }, 3000);
    }
    return () => clearTimeout(timer);
  }, [cognitiveState.stuck, isWallActive]);

  // 4. Sensor Integration (Unlock Logic)
  useEffect(() => {
    const unlisten = listen("sensor-accelerometer", (event) => {
      console.log("Sensor Event:", event.payload);
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

  // Default to Main Dashboard
  return (
    <Dashboard cognitiveState={cognitiveState} />
  );
}

export default App;
