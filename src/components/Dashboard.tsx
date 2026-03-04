import React, { useEffect, useState } from 'react';
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

  // Session elapsed timer (1Hz)
  useEffect(() => {
    const interval = setInterval(() => {
      setSessionSeconds((prev) => prev + 1);
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

  const formatSessionTime = (secs: number) => {
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return `${m}:${s < 10 ? '0' : ''}${s}`;
  };

  const formatKeyboardStatus = () => {
    if (keyboardIdleMs === 0) return 'Waiting...';
    if (keyboardIdleMs < 5000) return 'Active';
    return `Idle ${Math.floor(keyboardIdleMs / 1000)}s`;
  };

  const handleMonkModeToggle = () => {
    emit("monk-mode-change", !isMonkMode);
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
      <div className="session-info">
        <span className="session-time">Session: {formatSessionTime(sessionSeconds)}</span>
        <span className="keyboard-status">Keyboard: {formatKeyboardStatus()}</span>
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
        className={`monk-mode-button${isMonkMode ? ' active' : ''}`}
        onClick={handleMonkModeToggle}
      >
        {isMonkMode ? 'Monk Mode: ON — Wall disabled' : 'Monk Mode: OFF'}
      </button>

      <button className="quit-button" onClick={onQuit}>
        セッション終了
      </button>
    </div>
  );
};

export default Dashboard;
