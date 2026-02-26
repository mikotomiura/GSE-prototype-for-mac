use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::{bounded, Sender};

/// ログエントリの種別
#[derive(Debug)]
pub enum LogEntry {
    /// キーストロークイベント (生データ)
    Key {
        vk_code: u32,
        timestamp: u64,
        is_press: bool,
    },
    /// 特徴量 + HMM状態確率 (分析スレッドから)
    Feat {
        timestamp: u64,
        f1: f64,
        f2: f64,
        f3: f64,
        f4: f64,
        f5: f64,
        f6: f64,
        p_flow: f64,
        p_inc: f64,
        p_stuck: f64,
    },
    /// IME入力モードの切替イベント
    /// on=true  → 日本語入力モード (あ/カ) に移行 → β_writing を使用
    /// on=false → 英数直接入力モード (A) に移行   → β_coding を使用
    ImeState {
        timestamp: u64,
        on: bool,
    },
    /// セッション終了マーカー
    End,
}

/// NDJSON形式でセッションデータを書き込むロガー。
/// バックグラウンドスレッドでファイルIOを行うため、
/// 送信側 (分析スレッド) は非ブロッキングで送信できる。
pub struct SessionLogger {
    pub log_path: PathBuf,
    _handle: thread::JoinHandle<()>,
}

impl SessionLogger {
    /// ロガーを開始し、ログエントリ送信用の `Sender` を返す。
    /// `Sender` を Clone して複数スレッドから送信可能。
    pub fn start(log_path: PathBuf) -> (Self, Sender<LogEntry>) {
        let (tx, rx) = bounded::<LogEntry>(512);
        let path_clone = log_path.clone();

        let handle = thread::spawn(move || {
            // 親ディレクトリを作成
            if let Some(parent) = path_clone.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let file = match File::create(&path_clone) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("SessionLogger: failed to create {:?}: {}", path_clone, e);
                    return;
                }
            };

            let mut writer = BufWriter::new(file);

            // セッション開始メタデータ
            let session_start = now_ms();
            let _ = writeln!(
                writer,
                r#"{{"type":"meta","session_start":{}}}"#,
                session_start
            );

            tracing::info!("SessionLogger: writing to {:?}", path_clone);

            while let Ok(entry) = rx.recv() {
                match entry {
                    LogEntry::Key {
                        vk_code,
                        timestamp,
                        is_press,
                    } => {
                        let _ = writeln!(
                            writer,
                            r#"{{"type":"key","t":{},"vk":{},"press":{}}}"#,
                            timestamp,
                            vk_code,
                            is_press,
                        );
                    }
                    LogEntry::Feat {
                        timestamp,
                        f1,
                        f2,
                        f3,
                        f4,
                        f5,
                        f6,
                        p_flow,
                        p_inc,
                        p_stuck,
                    } => {
                        let _ = writeln!(
                            writer,
                            r#"{{"type":"feat","t":{},"f1":{:.2},"f2":{:.2},"f3":{:.4},"f4":{:.2},"f5":{:.1},"f6":{:.4},"p_flow":{:.4},"p_inc":{:.4},"p_stuck":{:.4}}}"#,
                            timestamp, f1, f2, f3, f4, f5, f6, p_flow, p_inc, p_stuck,
                        );
                    }
                    LogEntry::ImeState { timestamp, on } => {
                        let _ = writeln!(
                            writer,
                            r#"{{"type":"ime_state","t":{},"on":{}}}"#,
                            timestamp, on,
                        );
                    }
                    LogEntry::End => {
                        let _ = writeln!(
                            writer,
                            r#"{{"type":"meta","session_end":{}}}"#,
                            now_ms()
                        );
                        break;
                    }
                }

                // バッファを定期フラッシュ (エラー握りつぶし)
                let _ = writer.flush();
            }

            let _ = writer.flush();
            tracing::info!("SessionLogger: session closed");
        });

        let logger = Self {
            log_path,
            _handle: handle,
        };

        (logger, tx)
    }
}

/// UNIX時刻をミリ秒で返す
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Documents/GSE-sessions/gse_YYYYMMDD_HHMMSS.ndjson のパスを生成する
pub fn default_log_path() -> PathBuf {
    // Tauri の path API を使わず標準環境変数で取得 (lib.rs の setup 前に呼べるように)
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let dir = PathBuf::from(base)
        .join("Documents")
        .join("GSE-sessions");

    // タイムスタンプ付きファイル名
    let ts = chrono_like_filename();
    dir.join(format!("gse_{}.ndjson", ts))
}

/// chrono 非依存のタイムスタンプ文字列生成 (標準ライブラリのみ)
fn chrono_like_filename() -> String {
    // UNIX時刻から YYYYMMDD_HHMMSS を近似生成
    // 完全な精度は不要 (セッション識別に使うだけ)
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 2000-01-01 00:00:00 UTC からの秒数ベースで日時を概算
    // 注: 簡易実装のためタイムゾーンはUTC
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // グレゴリオ暦変換 (ユリウス通算日から)
    let jd = days_since_epoch + 2440588; // UNIX epoch = JD 2440588
    let a = jd + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m_cal = (5 * e + 2) / 153;
    let day = e - (153 * m_cal + 2) / 5 + 1;
    let month = m_cal + 3 - 12 * (m_cal / 10);
    let year = 100 * b + d - 4800 + m_cal / 10;

    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", year, month, day, h, m, s)
}
