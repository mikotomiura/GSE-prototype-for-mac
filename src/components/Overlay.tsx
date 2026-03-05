import React, { useEffect, useRef, useState, useMemo } from 'react';
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
    const [serverUrl, setServerUrl] = useState<string | null>(null);
    const [phoneConnected, setPhoneConnected] = useState(false);
    const [showTypingWarning, setShowTypingWarning] = useState(false);
    const currentWindow = useMemo(() => getCurrentWindow(), []);

    // Wall発動後の打鍵検知用 ref
    const wallStartTimeRef = useRef(0);
    const lastWarnedKeyAtRef = useRef(0);

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
            setServerUrl(null);
            setPhoneConnected(false);
            return;
        }

        // Wall 発動時: サーバー起動 + QR コード取得
        invoke<WallServerInfo>("start_wall_server")
            .then((info) => {
                setQrSvg(info.qr_svg);
                setServerUrl(info.url);
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

    // Wall発動後の打鍵検知 — Wall発動「後」の打鍵のみ警告 (False Positive防止)
    useEffect(() => {
        if (!isWallActive) {
            wallStartTimeRef.current = 0;
            lastWarnedKeyAtRef.current = 0;
            setShowTypingWarning(false);
            return;
        }

        // Wall発動時刻を記録
        wallStartTimeRef.current = Date.now();

        const interval = setInterval(() => {
            invoke<number>("get_last_keypress_timestamp").then((lastKeyAt) => {
                // Wall発動後の打鍵かつ、前回警告から1秒以上経過（デバウンス）
                if (
                    lastKeyAt > wallStartTimeRef.current &&
                    lastKeyAt > lastWarnedKeyAtRef.current + 1000
                ) {
                    lastWarnedKeyAtRef.current = lastKeyAt;
                    setShowTypingWarning(true);
                    // 2.5秒後に警告を非表示
                    setTimeout(() => setShowTypingWarning(false), 2500);
                }
            }).catch(() => {});
        }, 300);

        return () => clearInterval(interval);
    }, [isWallActive]);

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
                            {serverUrl && (
                                <p className="qr-url">{serverUrl}</p>
                            )}
                        </div>
                    ) : (
                        <p className="qr-loading">Starting unlock server...</p>
                    )}

                    <p className="wall-network-hint">
                        PCとスマホが同じWi-Fiに接続されていることを確認してください
                    </p>

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
