import React, { useEffect, useState, useMemo } from 'react';
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface OverlayProps {
    stuckProb: number;
    isWallActive: boolean;
    keyboardIdleMs: number;
}

interface WallServerInfo {
    qr_svg: string;
    url: string;
}

// 打鍵検知の閾値: Wall中にこの時間(ms)以内にキー入力があれば警告表示
const TYPING_WARN_THRESHOLD_MS = 3000;

const Overlay: React.FC<OverlayProps> = ({ stuckProb, isWallActive, keyboardIdleMs }) => {
    const [nudgeOpacity, setNudgeOpacity] = useState(0);
    const [qrSvg, setQrSvg] = useState<string | null>(null);
    const [phoneConnected, setPhoneConnected] = useState(false);
    const currentWindow = useMemo(() => getCurrentWindow(), []);

    // 透過背景 + 即座にクリック透過を設定（マウント直後）
    useEffect(() => {
        document.documentElement.style.background = 'transparent';
        document.body.style.background = 'transparent';
        currentWindow.setIgnoreCursorEvents(true);
    }, [currentWindow]);

    // isWallActive に応じてクリック透過をトグル
    // Lv1 (nudge): クリック透過 → ユーザーは下のウィンドウを操作可能
    // Lv2 (wall):  クリックブロック → 物理的に動くまで操作不可
    useEffect(() => {
        currentWindow.setIgnoreCursorEvents(!isWallActive);
    }, [isWallActive, currentWindow]);

    // スマートフォン接続検知
    useEffect(() => {
        const unlisten = listen("wall-phone-connected", () => {
            setPhoneConnected(true);
        });
        return () => { unlisten.then((f) => f()); };
    }, []);

    // Wall サーバーライフサイクル管理
    useEffect(() => {
        if (!isWallActive) {
            // Wall 解除時: サーバー停止 + 状態リセット
            invoke("stop_wall_server").catch(console.error);
            setQrSvg(null);
            setPhoneConnected(false);
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

    // Wall中の打鍵検知: keyboardIdleMs > 0 かつ閾値以内なら警告
    const showTypingWarning = isWallActive && keyboardIdleMs > 0 && keyboardIdleMs < TYPING_WARN_THRESHOLD_MS;

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

                    {phoneConnected && (
                        <div className="phone-connected-badge">
                            Phone connected — Zen Timer running
                        </div>
                    )}

                    {showTypingWarning && (
                        <div className="wall-typing-warning">
                            PCから離れてください — Step away from your PC
                        </div>
                    )}
                </div>
            )}
        </div>
    );
};

export default Overlay;
