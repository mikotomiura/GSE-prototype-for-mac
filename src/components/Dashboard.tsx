import React from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';

interface DashboardProps {
  cognitiveState: {
    flow: number;
    incubation: number;
    stuck: number;
  };
}

const Dashboard: React.FC<DashboardProps> = ({ cognitiveState }) => {
  // Determine dominant state for color coding
  const getMaxState = () => {
    const { flow, incubation, stuck } = cognitiveState;
    if (flow >= incubation && flow >= stuck) return 'Flow';
    if (incubation >= flow && incubation >= stuck) return 'Incubation';
    return 'Stuck';
  };

  const dominant = getMaxState();

  // Window controls
  const appWindow = getCurrentWindow();

  const handleMinimize = () => {
    appWindow.minimize();
  };

  const handleClose = () => {
    appWindow.close();
  };

  return (
    <div className={`dashboard-container state-${dominant.toLowerCase()}`}>
      {/* Custom Title Bar */}
      <div className="title-bar">
        <div
          className="drag-region"
          onMouseDown={() => { appWindow.startDragging(); }}
        >
          GSE Next
        </div>
        <div className="window-controls">
          <div
            className="control-btn"
            onClick={() => handleMinimize()}
            onMouseDown={(e) => e.stopPropagation()}
          >
            &#8211; {/* Minimize symbol */}
          </div>
          <div
            className="control-btn close"
            onClick={() => handleClose()}
            onMouseDown={(e) => e.stopPropagation()}
          >
            &#10005; {/* Close symbol */}
          </div>
        </div>
      </div>

      <div className="dashboard-content">
        <h2>Cognitive State Engine</h2>

        <div className="state-card">
          <h3>Current State: <span className="dominant-state">{dominant}</span></h3>
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

        {/* System Exit Button */}
        <button
          onClick={async () => {
            console.log("System Exit clicked");
            await invoke("quit_app");
          }}
          style={{
            marginTop: '10px',
            background: '#ef4444',
            color: 'white',
            border: 'none',
            padding: '8px 16px',
            borderRadius: '6px',
            cursor: 'pointer',
            zIndex: 9999,
            pointerEvents: 'auto',
            fontWeight: 'bold',
            boxShadow: '0 2px 4px rgba(0,0,0,0.2)'
          }}
        >
          システム終了 (System Exit)
        </button>
      </div>
    </div>
  );
};

export default Dashboard;
