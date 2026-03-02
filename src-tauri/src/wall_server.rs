// Smartphone-based Wall unlock via embedded HTTP server + QR code.
//
// When the Wall (Lv2 Stuck intervention) activates, an HTTP server starts on
// a random port and serves a self-contained Zen Timer page (2-minute countdown).
// The Overlay displays a QR code pointing to this page.
//
// Flow:
//   1. Overlay calls `start_wall_server` → server starts, returns QR SVG
//   2. User scans QR → phone opens Zen Timer page → server emits `wall-phone-connected`
//   3. After 120 s countdown (or early exit), phone POSTs /unlock?token=xxx
//   4. Server emits `sensor-accelerometer` / `"move"` → Wall dismissed
//   5. 120-second auto-unlock fallback if phone unavailable (synced with phone timer)

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use qrcode::QrCode;
use tauri::{Emitter, Runtime};
use tiny_http::{Header, Response, Server};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Information returned to the frontend when the wall server starts.
#[derive(serde::Serialize, Clone, Debug)]
pub struct WallServerInfo {
    /// QR code as a data:image/svg+xml;base64,... URL for <img src>
    pub qr_svg: String,
    /// The URL the smartphone should visit (for debug display)
    pub url: String,
}

/// Manages the lifecycle of the wall unlock HTTP server.
pub struct WallServer {
    shutdown: Arc<AtomicBool>,
    info: WallServerInfo,
    _handle: thread::JoinHandle<()>,
}

impl WallServer {
    /// Start the unlock server. Binds to 0.0.0.0:0 (OS-assigned port),
    /// generates a session token and QR code, and spawns the server thread.
    pub fn start<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(Self, WallServerInfo), String> {
        let server = Server::http("0.0.0.0:0").map_err(|e| format!("HTTP bind failed: {}", e))?;

        let port = server
            .server_addr()
            .to_ip()
            .map(|addr| addr.port())
            .ok_or("Failed to get server port")?;

        let local_ip = detect_local_ip();
        let token = generate_token();
        let url = format!("http://{}:{}/shake?token={}", local_ip, port, token);

        tracing::info!("WallServer: started on {}:{} (token={}...)", local_ip, port, &token[..8]);

        let qr_svg = generate_qr_data_url(&url)?;

        let info = WallServerInfo {
            qr_svg,
            url: url.clone(),
        };

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let info_clone = info.clone();
        let server_arc = Arc::new(server);

        let handle = thread::spawn(move || {
            server_loop(server_arc, token, shutdown_clone, app);
        });

        let info_for_return = info_clone.clone();

        let wall_server = Self {
            shutdown,
            info: info_clone,
            _handle: handle,
        };

        Ok((wall_server, info_for_return))
    }

    /// Get a clone of the server info (for returning to frontend on duplicate calls).
    pub fn info(&self) -> &WallServerInfo {
        &self.info
    }

    /// Signal the server thread to shut down.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Server loop
// ---------------------------------------------------------------------------

fn server_loop<R: Runtime>(
    server: Arc<Server>,
    token: String,
    shutdown: Arc<AtomicBool>,
    app: tauri::AppHandle<R>,
) {
    let start = Instant::now();
    let fallback_timeout = Duration::from_secs(120);

    // Build the server URL for embedding in the HTML page
    let port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .unwrap_or(0);
    let local_ip = detect_local_ip();
    let server_url = format!("http://{}:{}", local_ip, port);

    loop {
        // 60-second auto-unlock fallback
        if start.elapsed() > fallback_timeout {
            tracing::info!("WallServer: 120s timeout — auto-unlocking wall");
            let _ = app.emit("sensor-accelerometer", "move");
            break;
        }

        if shutdown.load(Ordering::Acquire) {
            break;
        }

        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                dispatch_request(request, &token, &server_url, &shutdown, &app);
            }
            Ok(None) => { /* timeout, loop again */ }
            Err(e) => {
                tracing::warn!("WallServer: recv error: {}", e);
                break;
            }
        }
    }

    tracing::info!("WallServer: server thread exiting");
}

fn dispatch_request<R: Runtime>(
    request: tiny_http::Request,
    token: &str,
    server_url: &str,
    shutdown: &Arc<AtomicBool>,
    app: &tauri::AppHandle<R>,
) {
    let url = request.url().to_string();

    // GET /shake?token=xxx — serve the Zen Timer HTML page
    if url.starts_with("/shake") && extract_token(&url) == token {
        let html = build_shake_html(server_url, token);
        let header =
            Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
        let response = Response::from_string(html).with_header(header);
        let _ = request.respond(response);

        // Notify PC frontend that a phone has connected
        let _ = app.emit("wall-phone-connected", ());
        tracing::info!("WallServer: smartphone connected (Zen Timer page served)");
    }
    // POST /unlock?token=xxx — unlock the wall
    else if url.starts_with("/unlock") && extract_token(&url) == token {
        tracing::info!("WallServer: unlock signal received from smartphone");
        let _ = app.emit("sensor-accelerometer", "move");
        shutdown.store(true, Ordering::Release);

        let cors = Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap();
        let ct = Header::from_bytes("Content-Type", "application/json").unwrap();
        let response = Response::from_string(r#"{"status":"unlocked"}"#)
            .with_header(cors)
            .with_header(ct);
        let _ = request.respond(response);
    }
    // OPTIONS (CORS preflight for POST)
    else if request.method() == &tiny_http::Method::Options {
        let cors1 = Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap();
        let cors2 = Header::from_bytes("Access-Control-Allow-Methods", "POST, OPTIONS").unwrap();
        let cors3 =
            Header::from_bytes("Access-Control-Allow-Headers", "Content-Type").unwrap();
        let response = Response::from_string("")
            .with_header(cors1)
            .with_header(cors2)
            .with_header(cors3);
        let _ = request.respond(response);
    }
    // Token mismatch
    else if url.starts_with("/shake") || url.starts_with("/unlock") {
        let response =
            Response::from_string("Forbidden").with_status_code(tiny_http::StatusCode(403));
        let _ = request.respond(response);
    }
    // Everything else
    else {
        let response =
            Response::from_string("Not Found").with_status_code(tiny_http::StatusCode(404));
        let _ = request.respond(response);
    }
}

/// Extract the token parameter from a URL like "/shake?token=abc123"
fn extract_token(url: &str) -> &str {
    url.split("token=").nth(1).unwrap_or("").split('&').next().unwrap_or("")
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Detect the local LAN IP address using the UDP socket trick.
/// Connects a UDP socket to an external address (no packet sent) and reads local_addr.
fn detect_local_ip() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("8.8.8.8:80")?;
            let addr = socket.local_addr()?;
            Ok(addr.ip().to_string())
        })
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

/// Generate a 32-character hex token using std RandomState as entropy source.
/// Sufficient for a local-network, single-session use case.
fn generate_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let s1 = RandomState::new();
    let mut h1 = s1.build_hasher();
    h1.write_u64(nanos);
    let v1 = h1.finish();

    let s2 = RandomState::new();
    let mut h2 = s2.build_hasher();
    h2.write_u64(v1 ^ std::process::id() as u64);
    let v2 = h2.finish();

    format!("{:016x}{:016x}", v1, v2)
}

/// Generate a QR code as a data:image/svg+xml;base64,... URL.
fn generate_qr_data_url(url: &str) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let code = QrCode::new(url.as_bytes()).map_err(|e| format!("QR encode failed: {}", e))?;

    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();

    let b64 = STANDARD.encode(svg.as_bytes());
    Ok(format!("data:image/svg+xml;base64,{}", b64))
}

// ---------------------------------------------------------------------------
// Shake detection HTML page (served to smartphone)
// ---------------------------------------------------------------------------

fn build_shake_html(server_url: &str, token: &str) -> String {
    SHAKE_HTML_TEMPLATE
        .replace("__SERVER_URL__", server_url)
        .replace("__TOKEN__", token)
}

const SHAKE_HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0, user-scalable=no">
<title>GSE - Zen Timer</title>
<style>
*{margin:0;padding:0;box-sizing:border-box}
body{font-family:-apple-system,BlinkMacSystemFont,sans-serif;background:#0f172a;color:#f8fafc;display:flex;flex-direction:column;align-items:center;justify-content:center;height:100vh;height:100dvh;padding:2rem;text-align:center;user-select:none;}
h1{font-size:1.5rem;margin-bottom:1rem;font-weight:600}
.sub{font-size:1rem;opacity:0.8;margin-bottom:2rem;line-height:1.5}
.timer-circle{width:150px;height:150px;border-radius:50%;border:4px solid rgba(255,255,255,0.1);display:flex;align-items:center;justify-content:center;font-size:3rem;font-weight:300;margin-bottom:2rem;position:relative;}
.timer-circle::after{content:'';position:absolute;top:-4px;left:-4px;right:-4px;bottom:-4px;border-radius:50%;border:4px solid transparent;border-top-color:#4ade80;animation:spin 2s linear infinite;opacity:0;}
.timer-circle.active::after{opacity:1;}
@keyframes spin{0%{transform:rotate(0deg)}100%{transform:rotate(360deg)}}
.btn{padding:1rem 2.5rem;font-size:1.1rem;background:#4ade80;color:#0f172a;border:none;border-radius:12px;cursor:pointer;font-weight:700;transition:transform 0.1s;}
.btn:active{transform:scale(0.95)}
.btn.hidden{display:none}
.btn-skip{padding:0.6rem 1.5rem;font-size:0.85rem;background:transparent;color:rgba(255,255,255,0.45);border:1px solid rgba(255,255,255,0.2);border-radius:8px;cursor:pointer;margin-top:1.5rem;transition:opacity 0.2s;}
.btn-skip:active{opacity:0.7}
.btn-skip.hidden{display:none}
.status{font-size:1.1rem;color:#4ade80;font-weight:600;display:none}
</style>
</head>
<body>
<h1>Take a Break</h1>
<p class="sub" id="msg">PCから離れて深呼吸してください。<br>2分後にロック解除ボタンが現れます。</p>
<div class="timer-circle" id="circle"><span id="time">02:00</span></div>
<button class="btn hidden" id="unlockBtn">Unlock Wall</button>
<button class="btn-skip" id="skipBtn">仕事に戻る — Return to work</button>
<p class="status" id="status">Unlocked! Return to your PC.</p>

<script>
(function(){
  var WAIT_SECONDS = 120;
  var SERVER='__SERVER_URL__',TOKEN='__TOKEN__';
  var timeEl = document.getElementById('time');
  var circleEl = document.getElementById('circle');
  var btnEl = document.getElementById('unlockBtn');
  var skipEl = document.getElementById('skipBtn');
  var msgEl = document.getElementById('msg');
  var statusEl = document.getElementById('status');
  var unlocked = false;

  circleEl.classList.add('active');

  var timer = setInterval(function(){
    WAIT_SECONDS--;
    var m = Math.floor(WAIT_SECONDS / 60);
    var s = WAIT_SECONDS % 60;
    timeEl.textContent = '0' + m + ':' + (s < 10 ? '0'+s : s);

    if(WAIT_SECONDS <= 0){
      clearInterval(timer);
      circleEl.classList.remove('active');
      circleEl.style.borderColor = '#4ade80';
      timeEl.textContent = '00:00';
      msgEl.innerHTML = '思考の整理はできましたか？<br>ボタンを押して再開してください。';
      btnEl.classList.remove('hidden');
      skipEl.classList.add('hidden');
    }
  }, 1000);

  function doUnlock(){
    if(unlocked) return;
    unlocked = true;
    btnEl.classList.add('hidden');
    skipEl.classList.add('hidden');
    statusEl.style.display = 'block';
    clearInterval(timer);
    fetch(SERVER+'/unlock?token='+TOKEN,{method:'POST'});
  }

  btnEl.addEventListener('click', doUnlock);
  skipEl.addEventListener('click', doUnlock);
})();
</script>
</body>
</html>"##;
