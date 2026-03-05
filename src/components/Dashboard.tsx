import React, { useEffect, useRef, useState } from 'react';
import { emit } from "@tauri-apps/api/event";

interface DashboardProps {
  cognitiveState: {
    flow: number;
    incubation: number;
    stuck: number;
  };
  onQuit: () => void;
  hookActive: boolean | null;
  keyboardIdleMs: number;
  isMonkMode: boolean;
}

const Dashboard: React.FC<DashboardProps> = ({ cognitiveState, onQuit, hookActive, keyboardIdleMs, isMonkMode }) => {
  const [sessionSeconds, setSessionSeconds] = useState(0);
  const sessionStartRef = useRef(Date.now());

  // Session elapsed timer (1Hz) — 絶対時間ベースでドリフトを防止
  useEffect(() => {
    const interval = setInterval(() => {
      setSessionSeconds(Math.floor((Date.now() - sessionStartRef.current) / 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, []);

  const getMaxState = () => {
    const { flow, incubation, stuck } = cognitiveState;
    if (flow >= incubation && flow >= stuck) return 'Flow';
    if (incubation >= flow && incubation >= stuck) return 'Incubation';
    return 'Stuck';
  };

  const dominant = getMaxState();

  const formatTime = (seconds: number) => {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    return `${m}:${String(s).padStart(2, '0')}`;
  };

  const keyIsActive = keyboardIdleMs > 0 && keyboardIdleMs < 3000;
  const keyStatusLabel = keyboardIdleMs === 0 ? '---' : keyIsActive ? 'Active' : `Idle ${Math.floor(keyboardIdleMs / 1000)}s`;

  const handleMonkModeToggle = () => {
    emit("monk-mode-changed", !isMonkMode);
  };

  return (
    <div className={`dashboard-container state-${dominant.toLowerCase()}`}>
      <h2>Generative Struggle Engine</h2>

      {/* Input Monitoring 権限バナー — macOS で権限未付与の場合のみ表示 */}
      {hookActive === false && (
        <div className="permission-banner">
          <span className="permission-icon">⚠️</span>
          <div className="permission-text">
            <strong>Input Monitoring 権限が必要です</strong>
            <p>
              System Settings が自動で開かれました。<br />
              <em>プライバシーとセキュリティ › 入力監視</em> で
              <strong> gse-next </strong>にチェックを入れ、
              アプリを再起動してください。
            </p>
          </div>
        </div>
      )}

      <div className="state-card">
        <h3>Current State: <span className="dominant-state">{dominant}</span></h3>
      </div>

      {/* Session & Keyboard Status */}
      <div className="session-bar">
        <div className="session-item">
          <span className="session-label">Session</span>
          <span className="session-value">{formatTime(sessionSeconds)}</span>
        </div>
        <div className="session-item">
          <span className="session-label">Keyboard</span>
          <span className={`session-value kbd-status ${keyIsActive ? 'kbd-active' : 'kbd-idle'}`}>
            {keyStatusLabel}
          </span>
        </div>
      </div>

      <div className="metrics-container">
        <div className="metric-row">
          <label>Flow</label>
          <div className="progress-bar-bg">
            <div
              className="progress-bar-fill flow"
              style={{ width: `${cognitiveState.flow * 100}%` }}
            ></div>
          </div>
          <span>{(cognitiveState.flow * 100).toFixed(1)}%</span>
        </div>

        <div className="metric-row">
          <label>Incubation</label>
          <div className="progress-bar-bg">
            <div
              className="progress-bar-fill incubation"
              style={{ width: `${cognitiveState.incubation * 100}%` }}
            ></div>
          </div>
          <span>{(cognitiveState.incubation * 100).toFixed(1)}%</span>
        </div>

        <div className="metric-row">
          <label>Stuck</label>
          <div className="progress-bar-bg">
            <div
              className="progress-bar-fill stuck"
              style={{ width: `${cognitiveState.stuck * 100}%` }}
            ></div>
          </div>
          <span>{(cognitiveState.stuck * 100).toFixed(1)}%</span>
        </div>
      </div>

      <div className="info-box">
        <p>Type naturally. The engine analyzes your keystroke dynamics.</p>
        <p><strong>Incubation</strong> suggests pausing. <strong>Stuck</strong> suggests moving.</p>
      </div>

      <button
        className={`monk-mode-toggle ${isMonkMode ? 'on' : 'off'}`}
        onClick={handleMonkModeToggle}
      >
        <span>{isMonkMode ? 'Monk Mode: ON' : 'Monk Mode: OFF'}</span>
        <span className="monk-mode-label">
          {isMonkMode ? 'Wall intervention disabled' : 'Wall intervention active'}
        </span>
      </button>

      <button className="quit-button" onClick={onQuit}>
        セッション終了
      </button>
    </div>
  );
};

export default Dashboard;
