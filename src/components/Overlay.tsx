import React, { useEffect, useState, useMemo, useRef } from 'react';
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface OverlayProps {
    stuckProb: number;
    isWallActive: boolean;
}

interface WallServerInfo {
    qr_svg: string;
    url: string;
}

const Overlay: React.FC<OverlayProps> = ({ stuckProb, isWallActive }) => {
    const [nudgeOpacity, setNudgeOpacity] = useState(0);
    const [qrSvg, setQrSvg] = useState<string | null>(null);
    const [wallTimer, setWallTimer] = useState(60);
    const currentWindow = useMemo(() => getCurrentWindow(), []);
    const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

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

    // Wall サーバーライフサイクル管理
    useEffect(() => {
        if (!isWallActive) {
            // Wall 解除時: サーバー停止 + 状態リセット
            invoke("stop_wall_server").catch(console.error);
            setQrSvg(null);
            setWallTimer(60);
            if (timerRef.current) {
                clearInterval(timerRef.current);
                timerRef.current = null;
            }
            return;
        }

        // Wall 発動時: サーバー起動 + QR コード取得
        invoke<WallServerInfo>("start_wall_server")
            .then((info) => {
                setQrSvg(info.qr_svg);
            })
            .catch((err) => {
                console.error("Failed to start wall server:", err);
            });

        // 60秒カウントダウン (フォールバック解除の視覚フィードバック)
        setWallTimer(60);
        timerRef.current = setInterval(() => {
            setWallTimer((prev) => {
                if (prev <= 1) {
                    if (timerRef.current) clearInterval(timerRef.current);
                    return 0;
                }
                return prev - 1;
            });
        }, 1000);

        return () => {
            if (timerRef.current) {
                clearInterval(timerRef.current);
                timerRef.current = null;
            }
        };
    }, [isWallActive]);

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
                    <p>Scan the QR code with your phone to unlock</p>

                    {qrSvg ? (
                        <div className="qr-container">
                            <img src={qrSvg} alt="QR Code" className="qr-code" />
                            <p className="qr-hint">
                                Open camera app and point at QR code
                            </p>
                        </div>
                    ) : (
                        <p className="qr-loading">Starting unlock server...</p>
                    )}

                    <div className="wall-timer">
                        Auto-unlock in {wallTimer}s
                    </div>
                </div>
            )}
        </div>
    );
};

export default Overlay;
