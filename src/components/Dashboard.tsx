import React from 'react';

interface DashboardProps {
  cognitiveState: {
    flow: number;
    incubation: number;
    stuck: number;
  };
  onQuit: () => void;
}

const Dashboard: React.FC<DashboardProps> = ({ cognitiveState, onQuit }) => {
  // Determine dominant state for color coding
  const getMaxState = () => {
    const { flow, incubation, stuck } = cognitiveState;
    if (flow >= incubation && flow >= stuck) return 'Flow';
    if (incubation >= flow && incubation >= stuck) return 'Incubation';
    return 'Stuck';
  };

  const dominant = getMaxState();

  return (
    <div className={`dashboard-container state-${dominant.toLowerCase()}`}>
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

      <button className="quit-button" onClick={onQuit}>
        セッション終了
      </button>
    </div>
  );
};

export default Dashboard;
