# GSE Prototype v2 (Next)

GSE (Global Sensing Engine) Next は、キーストロークのダイナミクスとセンサーフュージョンを利用して、ユーザーの認知状態（Cognitive State）を監視・最適化するためのデスクトップアプリケーションプロトタイプです。

## 主な機能

- **Sensing Layer**: Windowsの低レベルフックを使用して、アプリケーションを問わずキーストロークのFlight Time（打鍵間隔）を計測します。
- **Inference Layer (HMM)**: 計測データから認知状態を推論します：
  - 🟢 **Flow**: 没入・集中状態（生産性が高い）。
  - 🟡 **Incubation**: 思考中・停滞の兆候（休憩が必要かも）。
  - 🔴 **Stuck**: 行き詰まり状態（介入が必要）。
- **Intervention Layer**:
  - **Nudge**: 集中が切れかけた時に、画面端に赤いグロー効果でさりげなく通知。
  - **The Wall**: "Stuck" 状態が続くと全画面オーバーレイで強制的に作業を中断させます。
  - **IME Monitor**: 日本語入力（変換候補選択中）の停滞を「迷い」として誤検知しないよう、IMEの状態を監視します。
- **Sensor Fusion (Surface Pro 8)**:
  - **Accelerometer**: ユーザーの移動（歩行など）を検知して "The Wall" を解除します。
  - **Geolocation**: 場所の変化を記録します。

## 技術スタック

- **Frontend**: React, TypeScript, Vite
- **Backend**: Rust, Tauri v2 (Windows API / WinRT)
- **OS**: Windows 10/11 (必須)

## アーキテクチャ

**Core-Shell** アーキテクチャを採用しています：
- **Core (Rust)**: フック、HMM推論、センサー制御などの高負荷・システム依存処理を担当。
- **Shell (React)**: 状態の可視化とオーバーレイUIを担当。

## 開発環境のセットアップ

### 前提条件
- Node.js (v18以上)
- Rust (Stable)
- Windows SDK

### ビルドと実行
```bash
# 依存関係のインストール
npm install

# 開発モードで実行
npm run tauri dev
```
