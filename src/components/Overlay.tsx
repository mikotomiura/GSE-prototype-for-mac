import React, { useEffect, useState, useMemo } from 'react';
import { getCurrentWindow } from "@tauri-apps/api/window";

interface OverlayProps {
    stuckProb: number;
    isWallActive: boolean;
}

const Overlay: React.FC<OverlayProps> = ({ stuckProb, isWallActive }) => {
    const [nudgeOpacity, setNudgeOpacity] = useState(0);
    const currentWindow = useMemo(() => getCurrentWindow(), []);

    // 透過背景を設定（overlay ウィンドウ用）
    useEffect(() => {
        document.documentElement.style.background = 'transparent';
        document.body.style.background = 'transparent';
    }, []);

    // isWallActive に応じてクリック透過をトグル
    // Lv1 (nudge): クリック透過 → ユーザーは下のウィンドウを操作可能
    // Lv2 (wall):  クリックブロック → 物理的に動くまで操作不可
    useEffect(() => {
        currentWindow.setIgnoreCursorEvents(!isWallActive);
    }, [isWallActive, currentWindow]);

    useEffect(() => {
        // Nudge Logic: visual feedback starts at 0.6 stuck probability
        if (stuckProb > 0.6 && !isWallActive) {
            // Map 0.6-0.9 to 0.0-1.0 opacity
            const opacity = Math.min(1, (stuckProb - 0.6) / 0.3);
            setNudgeOpacity(opacity);
        } else {
            setNudgeOpacity(0);
        }
    }, [stuckProb, isWallActive]);

    return (
        <div className="overlay-root">
            {/* Nudge Layer (Red Vignette) */}
            <div
                className="nudge-layer"
                style={{ opacity: nudgeOpacity }}
            ></div>

            {/* The Wall (Blocking Layer) */}
            {isWallActive && (
                <div className="wall-layer">
                    <h1>Time to Move!</h1>
                    <p>Please stand up and walk around to unlock.</p>
                    <div className="scramble-animation">
                        {/* Abstract visual or icon */}
                        <span>🚶‍♂️ 🏃‍♂️ 🚶‍♂️</span>
                    </div>
                </div>
            )}
        </div>
    );
};

export default Overlay;
