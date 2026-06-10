// warp 的 filter 鏈是巨型嵌套型別,加上 rate_limit() 前置 filter 後超過預設遞迴上限
#![recursion_limit = "256"]
mod agent;
mod apply;
mod board_types;
mod cleanup;
mod layout;
mod llm;
mod prompts;
mod store;
mod stt;
mod sync;
mod yval;

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::Filter;

// the built client is embedded into the binary => one self-contained file (also what
// the Tauri webview loads). Requires `npm run build` (client/dist) before cargo build.
static CLIENT_ASSETS: include_dir::Dir =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/../client/dist");
fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn sanitize_ext(s: &str) -> String {
    let e: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if e.is_empty() {
        "webm".into()
    } else {
        e
    }
}
async fn write_tmp(prefix: &str, ext: &str, body: &[u8]) -> String {
    let p = std::env::temp_dir().join(format!("{}-{}.{}", prefix, store::rid(), ext));
    let _ = tokio::fs::write(&p, body).await;
    p.to_string_lossy().to_string()
}

// mori-desktop BodyManifest self-register (like mori-meeting-recorder), OFF by default
// so we never autonomously write the shared ~/.mori/. Enable with MORI_CANVAS_REGISTER=1.
fn maybe_register_body_part(port: u16) {
    if std::env::var("MORI_CANVAS_REGISTER").as_deref() != Ok("1") {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = format!("{}/.mori/body-parts/mori.canvas", home);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let manifest = json!({
        "schema_version": 1,
        "id": "mori.canvas",
        "name": "Mori Canvas",
        "kind": "local_service",
        "description": "會議共筆白板 — AI 把語音/逐字稿整理成便利貼+圖,多人即時協作。",
        "capabilities": ["whiteboard.collaborate", "meeting.visualize", "transcribe.local"],
        "entrypoints": { "web": format!("http://127.0.0.1:{}/", port) },
        "interfaces": [
            { "name": "api", "transport": "http", "base_url": format!("http://127.0.0.1:{}", port) },
            { "name": "sync", "transport": "ws", "url": format!("ws://127.0.0.1:{}/sync", port) }
        ],
        "permissions": [],
        "data_policy": { "owns_raw_data": true, "default_ingestion": "off" }
    });
    if std::fs::write(
        format!("{}/manifest.json", dir),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .is_ok()
    {
        println!("registered mori-desktop body part: {}/manifest.json", dir);
    }
    write_agentos_descriptor(port);
}

/// AgentOS 服務發現檔 `~/.mori/mori-canvas-server.json` —— 讓 AgentOS 在 dispatch `meeting.visualize`
/// (http-service / json 模式)時用 `forward_json` 找到本機 server、POST 到 `/api/visualize`。
/// 格式對齊 whisper-server 契約(AgentOS 的 descriptor 解析器只認 host/port/inference_path/contract_version);
/// 與 whisper-server.json / mori-recorder-server.json 並存不衝突。**呼叫端決定要不要寫**(standalone 路徑
/// 由 `MORI_CANVAS_REGISTER=1` gate;桌面 app 啟動即寫)。
pub fn write_agentos_descriptor(port: u16) {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return;
    }
    let dir = format!("{}/.mori", home);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let desc = json!({
        "contract_version": 1,
        "host": "127.0.0.1",
        "port": port,
        "pid": std::process::id(),
        "inference_path": "/api/visualize"
    });
    if std::fs::write(
        format!("{}/mori-canvas-server.json", dir),
        serde_json::to_string_pretty(&desc).unwrap(),
    )
    .is_ok()
    {
        println!(
            "wrote AgentOS descriptor: {}/mori-canvas-server.json (port {})",
            dir, port
        );
    }
}

#[derive(Clone, serde::Serialize)]
struct Settings {
    spacing: f64,
    #[serde(rename = "autoTidy")]
    auto_tidy: bool,
    mode: String, // mori | custom
    #[serde(rename = "sttSource")]
    stt_source: String, // cloud | local
    #[serde(rename = "localOnly")]
    local_only: bool,
    #[serde(rename = "whisperUrl")]
    whisper_url: String,
}
/// LLM_LOCAL_ONLY env 解析(純函數供測試):"1" / "true" 視為鎖定本機模式。
fn parse_local_only_env(v: Option<&str>) -> bool {
    match v.map(str::trim) {
        Some(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        None => false,
    }
}
/// 部署層的本機模式鎖:LLM_LOCAL_ONLY=1 時 local_only 開機即 true 且鎖死。
/// 鎖定狀態只來自 env、不進 SETTINGS —— 沒有任何 API 能改它,重啟也不會退回雲端。
static LOCKED_LOCAL_ONLY: Lazy<bool> =
    Lazy::new(|| parse_local_only_env(std::env::var("LLM_LOCAL_ONLY").ok().as_deref()));
/// POST /api/settings 的 localOnly 變更決策(純函數供測試):
/// 鎖定部署不允許關閉本機模式;回傳 (新值, 拒絕時的錯誤訊息)。
fn apply_local_only_change(
    locked: bool,
    current: bool,
    requested: Option<bool>,
) -> (bool, Option<&'static str>) {
    match requested {
        Some(false) if locked => (current, Some("此部署鎖定本機模式")),
        Some(v) => (v, None),
        None => (current, None),
    }
}
static SETTINGS: Lazy<Mutex<Settings>> = Lazy::new(|| {
    // capability-aware defaults: no mori-ear (e.g. on a cloud host) => custom/cloud (Groq Whisper)
    let caps = stt::stt_capabilities();
    let has_ear = caps
        .get("moriEar")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_ws = caps
        .get("whisperServer")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Mutex::new(Settings {
        spacing: 1.0,
        auto_tidy: true,
        mode: if has_ear { "mori" } else { "custom" }.into(),
        stt_source: if has_ws { "local" } else { "cloud" }.into(),
        local_only: *LOCKED_LOCAL_ONLY,
        whisper_url: String::new(),
    })
});

/// 部署層管理 token(ADMIN_TOKEN env):設定後 POST /api/settings 與
/// POST /api/rooms/:room/end 需帶相符的 X-Admin-Token header,否則 401。
/// 空白字串視同未設定。
static ADMIN_TOKEN: Lazy<Option<String>> = Lazy::new(|| {
    std::env::var("ADMIN_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
});
/// 連線來源是否 loopback(127.0.0.1 / ::1 / IPv4-mapped ::ffff:127.x)。
/// 注意:Render 等有反向代理的部署 remote 是 proxy IP、永遠不是 loopback(訪客自動被鎖);
/// 反之「同一台主機上的 nginx 反代」會讓所有訪客看起來都是 loopback ——
/// 公開部署一律請設 ADMIN_TOKEN,別只靠這個判斷。
fn is_loopback_addr(addr: Option<std::net::SocketAddr>) -> bool {
    match addr.map(|a| a.ip()) {
        Some(std::net::IpAddr::V4(ip)) => ip.is_loopback(),
        Some(std::net::IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip
                    .to_ipv4_mapped()
                    .map(|v4| v4.is_loopback())
                    .unwrap_or(false)
        }
        None => false,
    }
}
/// 「真正的本機管理員」判定:socket 來源是 loopback **且**沒有經過 proxy(無 XFF)。
/// 關鍵:Render / 同主機 nginx 反代是用 loopback 連到 app,於是每個外部訪客的 socket
/// 看起來都是 loopback —— 只看 is_loopback_addr 會把全世界都當管理員。帶了 X-Forwarded-For
/// 就代表這是被轉發進來的請求,絕不是直連本機的管理員,host 級設定一律不放行(除非帶對 token)。
fn is_trusted_local(addr: Option<std::net::SocketAddr>, xff: &Option<String>) -> bool {
    let proxied = xff.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false);
    is_loopback_addr(addr) && !proxied
}
/// 設定/管理端點的鑑權等級(純函數供測試)。
#[derive(Debug, PartialEq, Clone, Copy)]
enum SettingsAccess {
    /// 全部欄位可改:帶對 token 的管理者,或未設 token 的單機 loopback
    Full,
    /// 僅個人偏好(spacing / autoTidy)可改,主機級欄位拒改
    PersonalOnly,
    /// 整個請求拒絕(HTTP 401)
    Denied,
}
impl SettingsAccess {
    /// 主機級欄位(whisperUrl / mode / sttSource / localOnly / groqApiKey)可否修改
    fn allows_host_fields(self) -> bool {
        self == SettingsAccess::Full
    }
    /// 個人偏好欄位(spacing / autoTidy)可否修改
    fn allows_personal_fields(self) -> bool {
        self != SettingsAccess::Denied
    }
}
/// 鑑權決策:
/// - 設了 ADMIN_TOKEN:X-Admin-Token 相符 => 全開;不符 / 沒帶 => 整個請求 401(loopback 也一樣)。
/// - 未設 token(單機自用):loopback => 全開(維持現狀);非 loopback => 只能改個人偏好。
fn settings_access(
    admin_token: Option<&str>,
    header_token: Option<&str>,
    is_loopback: bool,
) -> SettingsAccess {
    match admin_token {
        Some(t) => {
            if header_token == Some(t) {
                SettingsAccess::Full
            } else {
                SettingsAccess::Denied
            }
        }
        None if is_loopback => SettingsAccess::Full,
        None => SettingsAccess::PersonalOnly,
    }
}
/// 401 回應:設了 ADMIN_TOKEN 的部署,token 不對就整個請求拒絕
fn admin_locked_reply() -> warp::reply::Response {
    use warp::Reply;
    let mut res = warp::reply::json(&json!({
        "ok": false,
        "error": "此部署已鎖定主機設定(需要正確的 X-Admin-Token)",
        "adminLocked": true,
    }))
    .into_response();
    *res.status_mut() = warp::http::StatusCode::UNAUTHORIZED;
    res
}

// ---- demo guards: per-IP rate limit + sponsor banner (env-driven, off unless set) ----
fn demo_rate_per_min() -> Option<u32> {
    std::env::var("DEMO_RATE_PER_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
}
static RATE_HITS: Lazy<Mutex<HashMap<String, Vec<std::time::Instant>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// XFF 整串可被 client 偽造,只信「最後一跳」(最近的受信 proxy 附加的那筆);
/// 沒有 proxy(自架直連)就退回 socket 位址,限流才真的 per-IP。
fn client_ip(xff: &Option<String>, addr: &Option<std::net::SocketAddr>) -> String {
    if let Some(last) = xff
        .as_ref()
        .and_then(|s| s.split(',').next_back())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return last.to_string();
    }
    addr.map(|a| a.ip().to_string()).unwrap_or_else(|| "?".into())
}
/// sliding 60s window。回 None = 放行;回 Some(secs) = 超限,secs 後再試。No limit set => always ok.
async fn rate_wait(ip: &str) -> Option<u64> {
    let limit = match demo_rate_per_min() {
        Some(n) => n as usize,
        None => return None,
    };
    let now = std::time::Instant::now();
    let mut m = RATE_HITS.lock().await;
    let hits = m.entry(ip.to_string()).or_default();
    hits.retain(|t| now.duration_since(*t).as_secs() < 60);
    if hits.len() >= limit {
        let oldest = hits.iter().min().copied().unwrap_or(now);
        return Some(60u64.saturating_sub(now.duration_since(oldest).as_secs()).max(1));
    }
    hits.push(now);
    None
}
/// 限流回應:HTTP 429 + Retry-After,JSON 帶 retryAfterSeconds 讓 client 自動退避續傳
fn rate_limited_reply(wait: u64) -> warp::reply::Response {
    use warp::Reply;
    let mut res = warp::reply::json(&json!({
        "ok": false,
        "error": format!("太頻繁了,休息 {wait} 秒再試(demo 限流)"),
        "rateLimited": true,
        "retryAfterSeconds": wait,
    }))
    .into_response();
    *res.status_mut() = warp::http::StatusCode::TOO_MANY_REQUESTS;
    if let Ok(v) = warp::http::HeaderValue::from_str(&wait.to_string()) {
        res.headers_mut().insert("Retry-After", v);
    }
    res
}
#[derive(Debug)]
struct RateLimited(u64);
impl warp::reject::Reject for RateLimited {}
/// MAX_ROOMS 滿時的 custom rejection:recover 轉成 503 + 友善 JSON(client 直接顯示 error)
#[derive(Debug)]
struct RoomsFull(String);
impl warp::reject::Reject for RoomsFull {}
/// 所有端點開房的唯一入口:超過 MAX_ROOMS 的「全新房」轉成 RoomsFull rejection
async fn open_room(
    rooms: &sync::Rooms,
    name: &str,
) -> Result<Arc<sync::Room>, warp::Rejection> {
    sync::get_or_create_room(rooms, name)
        .await
        .map_err(|e| warp::reject::custom(RoomsFull(e)))
}
/// PUBLIC_ROOM_LIST env 解析(純函數供測試):未設或其他值 = 公開(自架現狀);
/// "0" / "false" = 房間清單只回數量、不回房號(demo 預設 — 房號即進房鑰匙)。
fn parse_public_room_list(v: Option<&str>) -> bool {
    match v.map(str::trim) {
        Some(s) => !(s == "0" || s.eq_ignore_ascii_case("false")),
        None => true,
    }
}
/// 掛在吃 AI/STT 的端點前面:超限就以 custom rejection 中斷,由 recover 轉成 429
fn rate_limit() -> impl Filter<Extract = (), Error = warp::Rejection> + Clone {
    warp::header::optional::<String>("x-forwarded-for")
        .and(warp::addr::remote())
        .and_then(
            |xff: Option<String>, addr: Option<std::net::SocketAddr>| async move {
                match rate_wait(&client_ip(&xff, &addr)).await {
                    Some(w) => Err(warp::reject::custom(RateLimited(w))),
                    None => Ok(()),
                }
            },
        )
        .untuple_one()
}
async fn handle_rejection(err: warp::Rejection) -> Result<warp::reply::Response, warp::Rejection> {
    if let Some(RateLimited(w)) = err.find::<RateLimited>() {
        return Ok(rate_limited_reply(*w));
    }
    if let Some(RoomsFull(msg)) = err.find::<RoomsFull>() {
        use warp::Reply;
        let mut res = warp::reply::json(&json!({ "ok": false, "error": msg, "roomsFull": true }))
            .into_response();
        *res.status_mut() = warp::http::StatusCode::SERVICE_UNAVAILABLE;
        return Ok(res);
    }
    Err(err)
}
fn sponsor_config() -> Value {
    let g = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    json!({
        "sponsorUrl": g("SPONSOR_URL"),
        "sponsorLabel": g("SPONSOR_LABEL").unwrap_or_else(|| "贊助".into()),
        "demoNotice": g("DEMO_NOTICE"),
    })
}

fn lan_ip() -> Option<String> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("1.1.1.1:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip().to_string())
}

fn with<T: Clone + Send>(
    t: T,
) -> impl Filter<Extract = (T,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || t.clone())
}

// visitor's "bring your own AI" override + output language, from request headers
// (set client-side; X-Lang: zh-TW|en — absent header keeps the zh-TW default)
fn llm_opts() -> impl Filter<Extract = (llm::LlmOpts,), Error = warp::Rejection> + Clone {
    warp::header::optional::<String>("x-llm-base")
        .and(warp::header::optional::<String>("x-llm-key"))
        .and(warp::header::optional::<String>("x-llm-model"))
        .and(warp::header::optional::<String>("x-lang"))
        .map(
            |base: Option<String>, key: Option<String>, model: Option<String>, lang: Option<String>| {
                llm::LlmOpts {
                    base,
                    key,
                    model,
                    lang: llm::Lang::parse(lang.as_deref()),
                }
            },
        )
}

fn room_title(default_type: &str, topic: &str, frames: &[Value], lang: llm::Lang) -> String {
    let board_word = if lang == llm::Lang::En { "Board" } else { "board" };
    if !frames.is_empty() {
        let label = if frames.len() == 1 {
            let typ = frames[0]
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or(default_type);
            board_types::label_lang(typ, lang)
        } else if lang == llm::Lang::En {
            "Multi-diagram board"
        } else {
            "多圖白板"
        };
        return format!(
            "{}:{}",
            label,
            if topic.is_empty() { board_word } else { topic }
        );
    }
    format!(
        "{}:{}",
        board_types::label_lang(default_type, lang),
        if topic.is_empty() { board_word } else { topic }
    )
}

fn card_owner(s: &Value) -> String {
    if let Some(o) = s.get("owner").and_then(|v| v.as_str()) {
        return format!("({})", o);
    }
    match s.get("drawnBy").and_then(|v| v.as_str()) {
        Some(d) if !["user", "agent", "voice", "bot"].contains(&d) => format!("({})", d),
        _ => String::new(),
    }
}

fn card_tags(s: &Value) -> String {
    s.get("tags")
        .and_then(|v| v.as_array())
        .map(|a| {
            let t: Vec<String> = a
                .iter()
                .filter_map(|x| x.as_str())
                .map(|x| format!("#{}", x))
                .collect();
            if t.is_empty() {
                String::new()
            } else {
                format!(" {}", t.join(" "))
            }
        })
        .unwrap_or_default()
}

fn card_text(shapes: &[Value], id: &str) -> String {
    shapes
        .iter()
        .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(id))
        .and_then(|s| s.get("text").and_then(|v| v.as_str()))
        .unwrap_or("?")
        .to_string()
}

fn board_section_markdown(
    heading: &str,
    type_key: &str,
    cards: &[&Value],
    conns: &[Value],
    shapes: &[Value],
    hlevel: &str,
    lang: llm::Lang,
) -> String {
    let bt = board_types::board_type(type_key);
    let order = ["blue", "green", "yellow", "red"];
    let mut by_cat: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for s in cards {
        let color = s.get("color").and_then(|v| v.as_str()).unwrap_or("yellow");
        let cat = board_types::color_label_lang(bt, color, lang)
            .unwrap_or_else(|| board_types::other_label(lang))
            .to_string();
        by_cat.entry(cat).or_default().push(format!(
            "- {}{}{}",
            s.get("text").and_then(|v| v.as_str()).unwrap_or(""),
            card_owner(s),
            card_tags(s)
        ));
    }
    let mut out = format!("{} {}\n", hlevel, heading);
    let mut cats: Vec<String> = order
        .iter()
        .filter_map(|c| board_types::color_label_lang(bt, c, lang))
        .map(|s| s.to_string())
        .collect();
    cats.push(board_types::other_label(lang).to_string());
    for cat in cats {
        if let Some(items) = by_cat.get(&cat) {
            if !items.is_empty() {
                out += &format!("\n**{}**\n{}\n", cat, items.join("\n"));
            }
        }
    }
    let ids: std::collections::HashSet<&str> = cards
        .iter()
        .filter_map(|c| c.get("id").and_then(|v| v.as_str()))
        .collect();
    let edges: Vec<String> = conns
        .iter()
        .filter(|c| {
            let f = c.get("from").and_then(|v| v.as_str()).unwrap_or("");
            let t = c.get("to").and_then(|v| v.as_str()).unwrap_or("");
            ids.contains(f) && ids.contains(t)
        })
        .map(|c| {
            format!(
                "- {} → {}",
                card_text(
                    shapes,
                    c.get("from").and_then(|v| v.as_str()).unwrap_or("?")
                ),
                card_text(shapes, c.get("to").and_then(|v| v.as_str()).unwrap_or("?"))
            )
        })
        .collect();
    if !edges.is_empty() {
        out += &format!(
            "\n**{}**\n{}\n",
            board_types::edge_label_lang(bt, lang),
            edges.join("\n")
        );
    }
    out + "\n"
}

fn export_markdown_from_parts(
    shapes: &[Value],
    conns: &[Value],
    frames: &[Value],
    mtype: &str,
    topic: &str,
    lang: llm::Lang,
) -> String {
    let section = |heading: &str, type_key: &str, cards: &[&Value], hlevel: &str| -> String {
        board_section_markdown(heading, type_key, cards, conns, shapes, hlevel, lang)
    };
    if !frames.is_empty() {
        let mut md = format!("# {}\n\n", room_title(mtype, topic, frames, lang));
        for f in frames {
            let fid = f.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let ftype = f.get("type").and_then(|v| v.as_str()).unwrap_or("meeting");
            let ftitle = f.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let fcards: Vec<&Value> = shapes
                .iter()
                .filter(|s| s.get("frameId").and_then(|v| v.as_str()) == Some(fid))
                .collect();
            md += &section(
                &format!("{}:{}", board_types::label_lang(ftype, lang), ftitle),
                ftype,
                &fcards,
                "##",
            );
        }
        md
    } else {
        let all: Vec<&Value> = shapes.iter().collect();
        section(
            &format!(
                "{}:{}",
                board_types::label_lang(mtype, lang),
                if topic.is_empty() { "board" } else { topic }
            ),
            mtype,
            &all,
            "#",
        )
    }
}

// frame-aware markdown export (port of /api/export)
fn export_markdown(room: &sync::Room, lang: llm::Lang) -> String {
    let shapes = store::read_map(room, "shapes");
    let conns = store::read_map(room, "connectors");
    let frames = store::frames_sorted(room);
    let (mtype, topic) = store::read_meta(room);
    export_markdown_from_parts(&shapes, &conns, &frames, &mtype, &topic, lang)
}

fn board_type_reference(type_key: &str) -> String {
    let bt = board_types::board_type(type_key);
    let colors = bt
        .colors
        .iter()
        .map(|(c, l)| format!("{}={}", c, l))
        .collect::<Vec<_>>()
        .join("、");
    format!(
        "{}({}):配色 {};連線={};規則={}",
        bt.key, bt.label, colors, bt.edge_label, bt.hint
    )
}

fn summary_board_input_from_parts(
    shapes: &[Value],
    conns: &[Value],
    frames: &[Value],
    mtype: &str,
    topic: &str,
) -> (String, String) {
    let title = room_title(mtype, topic, frames, llm::Lang::ZhTw);
    let mut refs: Vec<String> = Vec::new();
    let mut body = String::new();
    if frames.is_empty() {
        refs.push(board_type_reference(mtype));
        let cards: Vec<&Value> = shapes.iter().collect();
        body.push_str(&board_section_markdown(
            &title, mtype, &cards, conns, shapes, "##", llm::Lang::ZhTw,
        ));
    } else {
        for f in frames {
            let fid = f.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let ftype = f.get("type").and_then(|v| v.as_str()).unwrap_or("meeting");
            let ftitle = f.get("title").and_then(|v| v.as_str()).unwrap_or("");
            refs.push(format!(
                "圖框「{}」使用 {}",
                ftitle,
                board_type_reference(ftype)
            ));
            let cards: Vec<&Value> = shapes
                .iter()
                .filter(|s| s.get("frameId").and_then(|v| v.as_str()) == Some(fid))
                .collect();
            body.push_str(&board_section_markdown(
                &format!("{}:{}", board_types::board_type(ftype).label, ftitle),
                ftype,
                &cards,
                conns,
                shapes,
                "##",
                llm::Lang::ZhTw,
            ));
        }
        let unframed: Vec<&Value> = shapes
            .iter()
            .filter(|s| s.get("frameId").and_then(|v| v.as_str()).is_none())
            .collect();
        if !unframed.is_empty() {
            refs.push(format!(
                "未放入圖框的卡片使用 {}",
                board_type_reference(mtype)
            ));
            body.push_str(&board_section_markdown(
                "未分圖框",
                mtype,
                &unframed,
                conns,
                shapes,
                "##",
                llm::Lang::ZhTw,
            ));
        }
    }
    let input = format!(
        "白板標題:{}\n\n板型語意:\n{}\n\n白板內容(分類標題與連線標題已依板型轉換,不可改用預設會議語意):\n{}",
        title,
        refs.join("\n"),
        body
    );
    (title, input)
}

pub(crate) fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b > a {
            out.replace_range(a..b + "</think>".len(), "");
        } else {
            break;
        }
    }
    out.trim().to_string()
}

// one-page board summary via the LLM (port of /api/summary)
async fn summary_markdown(
    room: &sync::Room,
    name: &str,
    local_only: bool,
    llm: &llm::LlmOpts,
) -> String {
    let shapes: Vec<Value> = store::read_map(room, "shapes")
        .into_iter()
        .filter(|s| s.get("type").and_then(|v| v.as_str()) == Some("sticky"))
        .collect();
    let conns = store::read_map(room, "connectors");
    let frames = store::frames_sorted(room);
    let (mtype, topic) = store::read_meta(room);
    let title = room_title(
        &mtype,
        if topic.is_empty() { name } else { &topic },
        &frames,
        llm::Lang::ZhTw,
    );
    // 摘要文件的固定框字(標題/空板/失敗)跟著請求語言;板型區段標題仍為 zh(deterministic 轉換)
    let (h_summary, msg_empty, msg_fail) = match llm.lang {
        llm::Lang::En => ("Board summary", "(The board is empty)", "Summary failed"),
        llm::Lang::ZhTw => ("白板摘要", "(白板還沒有內容)", "摘要失敗"),
    };
    if shapes.is_empty() {
        return format!("# {}:{}\n\n{}\n", h_summary, title, msg_empty);
    }
    let (_title, board) = summary_board_input_from_parts(
        &shapes,
        &conns,
        &frames,
        &mtype,
        if topic.is_empty() { name } else { &topic },
    );
    let sys = llm::with_output_lang(prompts::prompt("summary"), llm.lang);
    match llm::chat(
        &[
            llm::Msg {
                role: "system",
                content: sys,
            },
            llm::Msg {
                role: "user",
                content: board,
            },
        ],
        false,
        local_only,
        llm,
    )
    .await
    {
        Ok((t, _)) => format!("# {}:{}\n\n{}\n", h_summary, title, strip_think(&t)),
        Err(e) => format!("{}:{}", msg_fail, e),
    }
}

// per-room serialization lock (replaces the global lock)
static ROOM_LOCKS: Lazy<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
async fn room_lock(name: &str) -> Arc<Mutex<()>> {
    ROOM_LOCKS
        .lock()
        .await
        .entry(name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 把整段逐字稿切成適合餵 agent 的塊:先按換行 / 句末標點切小單位,再併到約 200 字一塊,
/// 壓低 LLM 呼叫數(agent 每次只處理「一段」、最多 6 張卡,整段硬塞會被截掉)。
fn chunk_transcript(t: &str) -> Vec<String> {
    const MAX: usize = 200;
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for unit in t
        .split(['\n', '。', '!', '?', '!', '?', ';', ';'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if !cur.is_empty() && cur.chars().count() + unit.chars().count() > MAX {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push('。');
        }
        cur.push_str(unit);
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks
}

// 內嵌示範畫板(= client/public/examples/meeting.json,格式 mori-canvas/v1)— 編譯期進 binary,
// Render 重啟磁碟清空後 DEMO 房一樣種得回來。
static DEMO_BOARD_JSON: &str = include_str!("../../client/public/examples/meeting.json");

/// 常駐示範房 ?room=DEMO:啟動時 .data/DEMO.bin 不存在就從種子種出來;
/// 之後每小時重置回種子內容(防塗改)。TTL 清理對 DEMO 豁免(sync::ttl_exempt)。
fn spawn_demo_room(rooms: sync::Rooms) {
    tokio::spawn(async move {
        let data: Value = match serde_json::from_str(DEMO_BOARD_JSON) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("demo board seed json broken: {e}");
                return;
            }
        };
        let mut first = true;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await; // 第一次 tick 立即觸發 => 啟動時就檢查
            // 啟動那次:已有 DEMO 檔就尊重現有內容(等一小時後的重置);之後每小時必重置
            let seed_now = !first || !sync::room_file_exists(sync::DEMO_ROOM);
            let label = if first { "startup seed" } else { "hourly reset" };
            first = false;
            if !seed_now {
                continue;
            }
            match sync::get_or_create_room(&rooms, sync::DEMO_ROOM).await {
                Ok(room) => {
                    store::seed_board(&room, &data);
                    let spacing = SETTINGS.lock().await.spacing;
                    apply::tidy_board(&room, spacing);
                    println!("demo room {:?}: {label}", sync::DEMO_ROOM);
                }
                Err(e) => eprintln!("demo room seed failed: {e}"),
            }
        }
    });
}

pub async fn serve(port: u16) {
    let rooms = sync::new_rooms();
    sync::init_persistence(rooms.clone());
    sync::spawn_room_ttl_cleaner(rooms.clone()); // ROOM_TTL_HOURS=0(預設)時不啟動
    spawn_demo_room(rooms.clone());

    // --- websocket sync (any path; strips optional sync/ prefix) ---
    let rooms_ws = rooms.clone();
    let ws = warp::path::tail()
        .and(warp::query::<HashMap<String, String>>())
        .and(warp::ws())
        .and_then(
            move |tail: warp::path::Tail, q: HashMap<String, String>, ws: warp::ws::Ws| {
                let rooms = rooms_ws.clone();
                async move {
                    let mut name = tail.as_str().to_string();
                    if let Some(r) = name.strip_prefix("sync/") {
                        name = r.to_string();
                    }
                    let name = percent_encoding::percent_decode_str(&name)
                        .decode_utf8_lossy()
                        .to_string();
                    let room = open_room(&rooms, &name).await?;
                    // ?view=1 = 唯讀連結;?key= 房主鑰匙(鎖板時憑它寫入)
                    let readonly = q.get("view").map(|v| v == "1").unwrap_or(false);
                    let conn_key = q.get("key").filter(|k| !k.is_empty()).cloned();
                    Ok::<_, warp::Rejection>(
                        ws.on_upgrade(move |socket| sync::peer(socket, room, conn_key, readonly)),
                    )
                }
            },
        );

    let health = warp::get()
        .and(warp::path!("api" / "health"))
        .map(|| warp::reply::json(&json!({ "ok": true, "server": "rust" })));
    let lan = warp::get()
        .and(warp::path!("api" / "lan"))
        .map(|| warp::reply::json(&json!({ "ip": lan_ip() })));

    // GET /api/rooms — active rooms (shapes + online counts)。
    // PUBLIC_ROOM_LIST=0(demo 預設)時只回 count 不回房號 — 房號即進房鑰匙,不能列給任何人。
    let r_rooms = rooms.clone();
    let rooms_list = warp::get()
        .and(warp::path!("api" / "rooms"))
        .and(with(r_rooms))
        .and_then(|rooms: sync::Rooms| async move {
            let map = rooms.read().await;
            if !parse_public_room_list(std::env::var("PUBLIC_ROOM_LIST").ok().as_deref()) {
                return Ok::<_, warp::Rejection>(warp::reply::json(
                    &json!({ "ok": true, "count": map.len() }),
                ));
            }
            let mut out = vec![];
            for (id, room) in map.iter() {
                let shapes = store::read_map(room, "shapes").len();
                let online = room.online.load(std::sync::atomic::Ordering::Relaxed);
                out.push(json!({ "id": id, "shapes": shapes, "online": online }));
            }
            Ok(warp::reply::json(
                &json!({ "ok": true, "count": out.len(), "rooms": out }),
            ))
        });

    // POST /api/rooms/:room/tidy
    let r_tidy = rooms.clone();
    let tidy = warp::post()
        .and(warp::path!("api" / "rooms" / String / "tidy"))
        .and(with(r_tidy))
        .and_then(|name: String, rooms: sync::Rooms| async move {
            let room = open_room(&rooms, &name).await?;
            let (mtype, _topic) = store::read_meta(&room);
            let sp = SETTINGS.lock().await.spacing;
            let shapes = store::read_map(&room, "shapes");
            let conns = store::read_map(&room, "connectors");
            let frames = store::read_map(&room, "frames");
            let (pos, fsz) = layout::tidy(&mtype, &shapes, &conns, &frames, sp);
            store::apply_tidy(&room, &pos, &fsz);
            Ok::<_, warp::Rejection>(warp::reply::json(&json!({ "ok": true })))
        });

    // POST /api/rooms/:room/end —— 設了 ADMIN_TOKEN 的部署需相符的 X-Admin-Token;
    // 未設 token 維持現狀(單機/區網自用,任何人可結束房間)
    let r_end = rooms.clone();
    let end = warp::post()
        .and(warp::path!("api" / "rooms" / String / "end"))
        .and(warp::header::optional::<String>("x-admin-token"))
        .and(warp::addr::remote())
        .and(with(r_end))
        .and_then(
            |name: String,
             token: Option<String>,
             addr: Option<std::net::SocketAddr>,
             rooms: sync::Rooms| async move {
                use warp::Reply;
                let access =
                    settings_access(ADMIN_TOKEN.as_deref(), token.as_deref(), is_loopback_addr(addr));
                if access == SettingsAccess::Denied {
                    return Ok::<_, warp::Rejection>(admin_locked_reply());
                }
                let room = open_room(&rooms, &name).await?;
                store::clear_room(&room);
                Ok(warp::reply::json(&json!({ "ok": true })).into_response())
            },
        );

    // GET/POST /api/rooms/:room/meta
    let r_meta_g = rooms.clone();
    let meta_get = warp::get().and(warp::path!("api" / "rooms" / String / "meta")).and(with(r_meta_g)).and_then(|name: String, rooms: sync::Rooms| async move {
        let room = open_room(&rooms, &name).await?;
        let (typ, topic) = store::read_meta(&room);
        let locked = room.locked.load(std::sync::atomic::Ordering::Relaxed);
        let has_owner = room.owner_key.lock().ok().map(|g| g.is_some()).unwrap_or(false);
        Ok::<_, warp::Rejection>(warp::reply::json(&json!({ "ok": true, "type": typ, "topic": topic, "types": board_types::types_list(), "locked": locked, "hasOwner": has_owner })))
    });

    // POST /api/rooms/:room/claim — 建房的第一個 client 領房主鑰匙(已有房主就只回你是不是)
    let r_claim = rooms.clone();
    let claim = warp::post()
        .and(warp::path!("api" / "rooms" / String / "claim"))
        .and(warp::body::json())
        .and(with(r_claim))
        .and_then(|name: String, body: Value, rooms: sync::Rooms| async move {
            let key: String = body.get("key").and_then(|v| v.as_str()).unwrap_or("").chars().take(64).collect();
            if key.is_empty() || name == sync::DEMO_ROOM {
                return Ok::<_, warp::Rejection>(warp::reply::json(&json!({ "ok": false, "error": "無法認領這個房間" })));
            }
            let room = open_room(&rooms, &name).await?;
            let owner = {
                let mut g = room.owner_key.lock().unwrap();
                if g.is_none() {
                    *g = Some(key.clone());
                }
                g.clone()
            };
            sync::save_owner_state(&name, &room);
            Ok(warp::reply::json(&json!({ "ok": true, "owner": owner.as_deref() == Some(key.as_str()) })))
        });

    // POST /api/rooms/:room/lock — 房主鎖板/解鎖;鎖板後其他連線的寫入在 ws 層被丟棄
    let r_lock = rooms.clone();
    let lock_ep = warp::post()
        .and(warp::path!("api" / "rooms" / String / "lock"))
        .and(warp::body::json())
        .and(with(r_lock))
        .and_then(|name: String, body: Value, rooms: sync::Rooms| async move {
            let key = body.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let want = body.get("locked").and_then(|v| v.as_bool()).unwrap_or(true);
            let room = open_room(&rooms, &name).await?;
            let is_owner = room
                .owner_key
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .map(|o| o == key)
                .unwrap_or(false);
            if !is_owner {
                return Ok::<_, warp::Rejection>(warp::reply::json(
                    &json!({ "ok": false, "error": "只有建房的人(房主)能鎖定/解鎖這塊板" }),
                ));
            }
            room.locked.store(want, std::sync::atomic::Ordering::Relaxed);
            sync::save_owner_state(&name, &room);
            Ok(warp::reply::json(&json!({ "ok": true, "locked": want })))
        });
    let r_meta_p = rooms.clone();
    let meta_post = warp::post()
        .and(warp::path!("api" / "rooms" / String / "meta"))
        .and(warp::body::json())
        .and(with(r_meta_p))
        .and_then(|name: String, body: Value, rooms: sync::Rooms| async move {
            let room = open_room(&rooms, &name).await?;
            let typ = body.get("type").and_then(|v| v.as_str());
            let typ = typ.filter(|t| board_types::BOARD_TYPES.iter().any(|b| b.key == *t));
            let topic = body.get("topic").and_then(|v| v.as_str());
            store::set_meta(&room, typ, topic);
            let (t, tp) = store::read_meta(&room);
            Ok::<_, warp::Rejection>(warp::reply::json(
                &json!({ "ok": true, "type": t, "topic": tp }),
            ))
        });

    // GET/POST /api/rooms/:room/frames
    let r_frames_g = rooms.clone();
    let frames_get = warp::get()
        .and(warp::path!("api" / "rooms" / String / "frames"))
        .and(with(r_frames_g))
        .and_then(|name: String, rooms: sync::Rooms| async move {
            let room = open_room(&rooms, &name).await?;
            Ok::<_, warp::Rejection>(warp::reply::json(
                &json!({ "ok": true, "frames": store::frames_sorted(&room) }),
            ))
        });
    let r_frames_p = rooms.clone();
    let frames_post = warp::post()
        .and(warp::path!("api" / "rooms" / String / "frames"))
        .and(warp::body::json())
        .and(with(r_frames_p))
        .and_then(|name: String, body: Value, rooms: sync::Rooms| async move {
            let room = open_room(&rooms, &name).await?;
            let typ = body
                .get("type")
                .and_then(|v| v.as_str())
                .filter(|t| board_types::BOARD_TYPES.iter().any(|b| b.key == *t))
                .unwrap_or(board_types::DEFAULT_BOARD_TYPE);
            let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let f = store::create_frame(&room, typ, title);
            Ok::<_, warp::Rejection>(warp::reply::json(&json!({ "ok": true, "frame": f })))
        });

    // GET /api/export/:room
    let r_export = rooms.clone();
    let export = warp::get()
        .and(warp::path!("api" / "export" / String))
        .and(warp::query::<HashMap<String, String>>())
        .and(with(r_export))
        .and_then(|name: String, q: HashMap<String, String>, rooms: sync::Rooms| async move {
            // ?lang= 後備:這支會被 window.open 直接打開(帶不了 X-Lang header)
            let lang = llm::Lang::parse(q.get("lang").map(|s| s.as_str()));
            let room = open_room(&rooms, &name).await?;
            let md = export_markdown(&room, lang);
            Ok::<_, warp::Rejection>(warp::reply::with_header(
                md,
                "Content-Type",
                "text/markdown; charset=utf-8",
            ))
        });

    // GET /api/summary/:room — one-page meeting note via the LLM。
    // ?lang= 後備:這支會被 window.open 直接打開(帶不了 X-Lang header)
    let r_summary = rooms.clone();
    let summary = warp::get()
        .and(warp::path!("api" / "summary" / String))
        .and(warp::query::<HashMap<String, String>>())
        .and(with(r_summary))
        .and(llm_opts())
        .and_then(
            |name: String, q: HashMap<String, String>, rooms: sync::Rooms, mut llm: llm::LlmOpts| async move {
                if let Some(l) = q.get("lang") {
                    llm.lang = llm::Lang::parse(Some(l));
                }
                let room = open_room(&rooms, &name).await?;
                let lo = SETTINGS.lock().await.local_only;
                let md = summary_markdown(&room, &name, lo, &llm).await;
                Ok::<_, warp::Rejection>(warp::reply::with_header(
                    md,
                    "Content-Type",
                    "text/markdown; charset=utf-8",
                ))
            },
        );

    // GET/POST /api/settings —— adminLocked = 設了 ADMIN_TOKEN 或來源非 loopback,
    // 表示「不帶 token 的話主機級欄位被鎖」(讓設定頁能提示)
    let settings_get = warp::get().and(warp::path!("api" / "settings")).and(warp::addr::remote()).and_then(|addr: Option<std::net::SocketAddr>| async move {
        let s = SETTINGS.lock().await.clone();
        let admin_locked = ADMIN_TOKEN.is_some() || !is_loopback_addr(addr);
        let mut o = json!({ "ok": true, "spacing": s.spacing, "autoTidy": s.auto_tidy, "mode": s.mode, "sttSource": s.stt_source, "localOnly": s.local_only, "lockedLocalOnly": *LOCKED_LOCAL_ONLY, "adminLocked": admin_locked, "whisperUrl": s.whisper_url, "groqKey": llm::groq_key().is_some() });
        for src in [llm::config_info(), stt::stt_capabilities(), sponsor_config()] {
            if let (Value::Object(dst), Value::Object(m)) = (&mut o, src) {
                for (k, v) in m {
                    dst.insert(k, v);
                }
            }
        }
        Ok::<_, warp::Rejection>(warp::reply::json(&o))
    });
    let settings_post = warp::post().and(warp::path!("api" / "settings")).and(warp::header::optional::<String>("x-admin-token")).and(warp::addr::remote()).and(warp::header::optional::<String>("x-forwarded-for")).and(warp::body::json()).and_then(|token: Option<String>, addr: Option<std::net::SocketAddr>, xff: Option<String>, body: Value| async move {
        use warp::Reply;
        let access = settings_access(ADMIN_TOKEN.as_deref(), token.as_deref(), is_trusted_local(addr, &xff));
        // 設了 ADMIN_TOKEN 而 token 不對:整個請求 401(個人偏好也不放行)
        if !access.allows_personal_fields() {
            return Ok::<_, warp::Rejection>(admin_locked_reply());
        }
        let host_ok = access.allows_host_fields();
        let mut host_rejected = false; // 有人試圖改主機級欄位但被擋
        let mut s = SETTINGS.lock().await;
        // 個人偏好欄位:spacing / autoTidy(公開部署的訪客也可改)
        if let Some(v) = body.get("spacing").and_then(|v| v.as_f64()) {
            s.spacing = v.clamp(0.6, 2.0);
        }
        if let Some(v) = body.get("autoTidy").and_then(|v| v.as_bool()) {
            s.auto_tidy = v;
        }
        // 以下皆為主機級欄位:未設 token 時僅 loopback 來源可改
        if let Some(v) = body.get("mode").and_then(|v| v.as_str()) {
            if !host_ok {
                host_rejected = true;
            } else if v == "mori" || v == "custom" {
                s.mode = v.into();
            }
        }
        if let Some(v) = body.get("sttSource").and_then(|v| v.as_str()) {
            if !host_ok {
                host_rejected = true;
            } else if v == "cloud" || v == "local" {
                s.stt_source = v.into();
            }
        }
        // 鎖定部署(LLM_LOCAL_ONLY)不允許關閉本機模式 —— 其他欄位照常生效
        let mut lo_err = None;
        if body.get("localOnly").and_then(|v| v.as_bool()).is_some() && !host_ok {
            host_rejected = true;
        } else {
            let (lo, e) = apply_local_only_change(
                *LOCKED_LOCAL_ONLY,
                s.local_only,
                body.get("localOnly").and_then(|v| v.as_bool()),
            );
            s.local_only = lo;
            lo_err = e;
        }
        if let Some(v) = body.get("whisperUrl").and_then(|v| v.as_str()) {
            if !host_ok {
                host_rejected = true;
            } else {
                s.whisper_url = v.chars().take(200).collect();
            }
        }
        // 設定頁貼的 Groq key 是 server 級全域 —— 只留給單機桌面版(loopback)或管理者;
        // 公開部署的訪客請走 BYO header(key 只在自己瀏覽器,訪客之間不共用)
        if let Some(v) = body.get("groqApiKey").and_then(|v| v.as_str()) {
            if !host_ok {
                host_rejected = true;
            } else {
                llm::set_runtime_groq_key(v);
            }
        }
        let mut o = json!({ "ok": true, "spacing": s.spacing, "autoTidy": s.auto_tidy, "mode": s.mode, "sttSource": s.stt_source, "localOnly": s.local_only, "lockedLocalOnly": *LOCKED_LOCAL_ONLY, "adminLocked": !host_ok, "whisperUrl": s.whisper_url, "groqKey": llm::groq_key().is_some(), "moriEar": stt::stt_capabilities().get("moriEar").cloned().unwrap_or(json!(false)), "whisperServer": stt::stt_capabilities().get("whisperServer").cloned().unwrap_or(json!(false)) });
        if let Some(e) = lo_err {
            o["ok"] = json!(false);
            o["error"] = json!(e);
        }
        if host_rejected {
            o["ok"] = json!(false);
            o["error"] = json!("主機級設定已鎖定,僅本機(loopback)或管理者可修改");
        }
        Ok(warp::reply::json(&o).into_response())
    });

    // POST /api/agent/:room — the AI turn (intent classify -> command or content)
    let r_agent = rooms.clone();
    let agent_ep = warp::post()
        .and(warp::path!("api" / "agent" / String))
        .and(rate_limit())
        .and(warp::body::json())
        .and(with(r_agent))
        .and(llm_opts())
        .and_then(
            |name: String, body: Value, rooms: sync::Rooms, llm: llm::LlmOpts| async move {
                let transcript = body
                    .get("transcript")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if transcript.is_empty() {
                    return Ok::<_, warp::Rejection>(warp::reply::json(
                        &json!({ "ok": false, "error": "transcript required" }),
                    ));
                }
                let by: String = body
                    .get("by")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent")
                    .chars()
                    .take(24)
                    .collect();
                let room = open_room(&rooms, &name).await?;
                let s = SETTINGS.lock().await.clone();
                // stage-1 清稿(可帶 "cleanup": false 跳過):贅字/斷句先處理掉,別讓它進卡片
                let do_cleanup = body.get("cleanup").and_then(|v| v.as_bool()).unwrap_or(true);
                let (transcript, cleaned) = if do_cleanup {
                    cleanup::cleanup_transcript(&transcript, s.local_only, &llm).await
                } else {
                    (transcript, false)
                };
                if transcript.trim().is_empty() {
                    return Ok(warp::reply::json(
                        &json!({ "ok": true, "stickies": 0, "connectors": 0, "added": [], "skipped": if llm.lang == llm::Lang::En { "Only filler words" } else { "整段都是語助詞" } }),
                    ));
                }
                let _lk = room_lock(&name).await;
                let _guard = _lk.lock().await; // serialize agent turns (per-room race guard)
                let res = apply::run_agent_turn(
                    &room,
                    &transcript,
                    &by,
                    s.local_only,
                    s.auto_tidy,
                    s.spacing,
                    &llm,
                )
                .await;
                Ok(match res {
                    Ok(mut v) => {
                        v["cleaned"] = json!(cleaned);
                        warp::reply::json(&v)
                    }
                    Err(e) => warp::reply::json(&json!({ "ok": false, "error": e })),
                })
            },
        );

    // POST /api/transcribe — audio -> text (no agent)
    let transcribe_ep = warp::post()
        .and(warp::path!("api" / "transcribe"))
        .and(rate_limit())
        .and(warp::query::<HashMap<String, String>>())
        .and(warp::body::bytes())
        .and_then(
            |q: HashMap<String, String>, body: bytes::Bytes| async move {
                let ext = sanitize_ext(q.get("ext").map(|s| s.as_str()).unwrap_or("webm"));
                let tmp = write_tmp("t", &ext, &body).await;
                let s = SETTINGS.lock().await.clone();
                let r = stt::transcribe(&tmp, &s.mode, &s.stt_source, &s.whisper_url, s.local_only)
                    .await;
                let _ = tokio::fs::remove_file(&tmp).await;
                Ok::<_, warp::Rejection>(match r {
                    Ok(text) => warp::reply::json(&json!({ "ok": true, "text": text })),
                    Err(e) => warp::reply::json(&json!({ "ok": false, "error": e })),
                })
            },
        );

    // POST /api/voice/:room — audio -> STT -> agent turn
    let r_voice = rooms.clone();
    let voice_ep = warp::post()
        .and(warp::path!("api" / "voice" / String))
        .and(rate_limit())
        .and(warp::query::<HashMap<String, String>>())
        .and(warp::body::bytes())
        .and(with(r_voice))
        .and(llm_opts())
        .and_then(
            |name: String,
             q: HashMap<String, String>,
             body: bytes::Bytes,
             rooms: sync::Rooms,
             llm: llm::LlmOpts| async move {
                let ext = sanitize_ext(q.get("ext").map(|s| s.as_str()).unwrap_or("webm"));
                let tmp = write_tmp("voice", &ext, &body).await;
                let s = SETTINGS.lock().await.clone();
                let transcript =
                    stt::transcribe(&tmp, &s.mode, &s.stt_source, &s.whisper_url, s.local_only)
                        .await;
                let _ = tokio::fs::remove_file(&tmp).await;
                let transcript = match transcript {
                    Ok(t) => t,
                    Err(e) => {
                        return Ok::<_, warp::Rejection>(warp::reply::json(
                            &json!({ "ok": false, "error": e }),
                        ))
                    }
                };
                if transcript.trim().is_empty() {
                    return Ok(warp::reply::json(
                        &json!({ "ok": true, "transcript": "", "stickies": 0 }),
                    ));
                }
                // stage-1 清稿:語音逐字稿先清贅字/重斷句,卡片才不會抄進「嗯/那個/對對對」
                let raw_transcript = transcript.clone();
                let (transcript, cleaned) =
                    cleanup::cleanup_transcript(&transcript, s.local_only, &llm).await;
                if transcript.trim().is_empty() {
                    return Ok(warp::reply::json(
                        &json!({ "ok": true, "transcript": "", "rawTranscript": raw_transcript, "stickies": 0, "skipped": if llm.lang == llm::Lang::En { "Only filler words" } else { "整段都是語助詞" } }),
                    ));
                }
                let by: String = q
                    .get("by")
                    .map(|s| s.as_str())
                    .unwrap_or("voice")
                    .chars()
                    .take(24)
                    .collect();
                let room = open_room(&rooms, &name).await?;
                let _lk = room_lock(&name).await;
                let _guard = _lk.lock().await;
                let mut res = match apply::run_agent_turn(
                    &room,
                    &transcript,
                    &by,
                    s.local_only,
                    s.auto_tidy,
                    s.spacing,
                    &llm,
                )
                .await
                {
                    Ok(v) => v,
                    Err(e) => json!({ "ok": false, "error": e }),
                };
                res["transcript"] = json!(transcript);
                res["rawTranscript"] = json!(raw_transcript);
                res["cleaned"] = json!(cleaned);
                Ok(warp::reply::json(&res))
            },
        );

    // POST /api/card/:room/:cardId — dictate one card's text/tags/owner/kind
    let r_card = rooms.clone();
    let card_ep = warp::post().and(warp::path!("api" / "card" / String / String)).and(rate_limit()).and(warp::query::<HashMap<String, String>>()).and(warp::body::bytes()).and(with(r_card)).and(llm_opts()).and_then(
        |name: String, card_id: String, q: HashMap<String, String>, body: bytes::Bytes, rooms: sync::Rooms, llm: llm::LlmOpts| async move {
            let ext = sanitize_ext(q.get("ext").map(|s| s.as_str()).unwrap_or("webm"));
            let tmp = write_tmp("c", &ext, &body).await;
            let s = SETTINGS.lock().await.clone();
            // STT 失敗要回報而不是吞掉 —— 本機模式封鎖雲端時,使用者得知道是模式擋的、不是壞掉
            let transcript = stt::transcribe(&tmp, &s.mode, &s.stt_source, &s.whisper_url, s.local_only).await;
            let _ = tokio::fs::remove_file(&tmp).await;
            let transcript = match transcript {
                Ok(t) => t,
                Err(e) => return Ok::<_, warp::Rejection>(warp::reply::json(&json!({ "ok": false, "error": e }))),
            };
            let room = open_room(&rooms, &name).await?;
            let cur = apply::card_current(&room, &card_id);
            if cur.is_none() {
                return Ok::<_, warp::Rejection>(warp::reply::json(&json!({ "ok": false, "error": "card not found", "transcript": transcript })));
            }
            if transcript.trim().is_empty() {
                return Ok(warp::reply::json(&json!({ "ok": true, "transcript": "", "edit": {} })));
            }
            let (text, owner, tags) = cur.unwrap();
            let _lk = room_lock(&name).await; let _guard = _lk.lock().await;
            let edit = match agent::plan_card_edit(&transcript, &text, owner.as_deref(), tags.as_deref(), s.local_only, &llm).await {
                Ok(e) => e,
                Err(e) => return Ok(warp::reply::json(&json!({ "ok": false, "error": e, "transcript": transcript }))),
            };
            apply::apply_card_edit(&room, &card_id, &edit);
            let mut ej = serde_json::Map::new();
            if let Some(t) = &edit.text {
                ej.insert("text".into(), json!(t));
            }
            if let Some(t) = &edit.tags {
                ej.insert("tags".into(), json!(t));
            }
            if let Some(o) = &edit.owner {
                ej.insert("owner".into(), json!(o));
            }
            if let Some(c) = &edit.color {
                ej.insert("color".into(), json!(c));
            }
            Ok(warp::reply::json(&json!({ "ok": true, "transcript": transcript, "edit": ej })))
        },
    );

    // POST /api/visualize — headless「整段逐字稿 → 建板 → 匯出」。AgentOS dispatch meeting.visualize 走這支:
    // 切塊逐塊餵 agent 建板 → tidy → 回 markdown / summary + 一個能打開**繼續編輯**的 url(room 持久化;
    // html/png 匯出走 client)。body: { transcript(必), room?, board_type? }。
    let r_viz = rooms.clone();
    let visualize_ep = warp::post()
        .and(warp::path!("api" / "visualize"))
        .and(rate_limit())
        .and(warp::body::json())
        .and(with(r_viz))
        .and(llm_opts())
        .and_then(move |body: Value, rooms: sync::Rooms, mut llm: llm::LlmOpts| async move {
            let transcript = body.get("transcript").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if transcript.is_empty() {
                return Ok::<_, warp::Rejection>(warp::reply::json(&json!({ "ok": false, "error": "transcript required" })));
            }
            // headless JSON 呼叫端(AgentOS dispatch)用 body 帶 lang 也行,優先於 header
            if let Some(l) = body.get("lang").and_then(|v| v.as_str()) {
                llm.lang = llm::Lang::parse(Some(l));
            }
            let name = body
                .get("room")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("visualize-{}", store::rid()));
            let room = open_room(&rooms, &name).await?;
            if let Some(bt) = body.get("board_type").and_then(|v| v.as_str()) {
                store::set_meta(&room, Some(bt), None);
            }
            let s = SETTINGS.lock().await.clone();
            let _lk = room_lock(&name).await;
            let _guard = _lk.lock().await; // serialize per-room
            // stage-1 清稿:補回標點之後,下面 chunk_transcript 的句界切分才會準
            let (transcript, _cleaned) =
                if body.get("cleanup").and_then(|v| v.as_bool()).unwrap_or(true) {
                    cleanup::cleanup_transcript(&transcript, s.local_only, &llm).await
                } else {
                    (transcript, false)
                };
            let chunks = chunk_transcript(&transcript);
            let total = chunks.len();
            let mut turns = 0usize;
            for c in &chunks {
                // auto_tidy=false:逐塊建板別每塊都重排,最後統一 tidy 一次。
                if apply::run_agent_turn(&room, c, "AI", s.local_only, false, s.spacing, &llm).await.is_ok() {
                    turns += 1;
                }
            }
            apply::tidy_board(&room, s.spacing);
            let markdown = export_markdown(&room, llm.lang);
            let summary = summary_markdown(&room, &name, s.local_only, &llm).await;
            let cards = store::read_map(&room, "shapes").iter().filter(|x| x.get("type").and_then(|v| v.as_str()) == Some("sticky")).count();
            let frames = store::frames_sorted(&room).len();
            Ok(warp::reply::json(&json!({
                "ok": true,
                "room": name,
                "url": format!("http://127.0.0.1:{}/?room={}", port, name),
                "chunks": total,
                "turns": turns,
                "cards": cards,
                "frames": frames,
                "markdown": markdown,
                "summary": summary
            })))
        });

    let api = agent_ep
        .or(visualize_ep)
        .or(transcribe_ep)
        .or(voice_ep)
        .or(card_ep)
        .or(health)
        .or(lan)
        .or(rooms_list)
        .or(tidy)
        .or(end)
        .or(meta_get)
        .or(meta_post)
        .or(claim)
        .or(lock_ep)
        .or(frames_get)
        .or(frames_post)
        .or(export)
        .or(summary)
        .or(settings_get)
        .or(settings_post);

    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec![
            "Content-Type",
            "x-llm-base",
            "x-llm-key",
            "x-llm-model",
            "x-admin-token",
            "x-lang",
        ]);
    // serve the embedded client (single self-contained binary: client + sync + api on one port).
    // SPA fallback: unknown paths -> index.html.
    let serve_client = warp::get()
        .and(warp::path::full())
        .map(|p: warp::path::FullPath| {
            let raw = p.as_str().trim_start_matches('/');
            let (file, mime) = match (!raw.is_empty())
                .then(|| CLIENT_ASSETS.get_file(raw))
                .flatten()
            {
                Some(f) => (Some(f), mime_for(raw)),
                None => (
                    CLIENT_ASSETS.get_file("index.html"),
                    "text/html; charset=utf-8",
                ),
            };
            match file {
                Some(f) => warp::http::Response::builder()
                    .header("Content-Type", mime)
                    .body(f.contents().to_vec())
                    .unwrap(),
                None => warp::http::Response::builder()
                    .status(404)
                    .body(b"not found".to_vec())
                    .unwrap(),
            }
        });
    // recover 要掛在 api / ws 這層:掛在 serve_client 之後的話,GET 端點的 custom rejection
    // (RoomsFull)會先被 SPA fallback(任意 GET 都回 index.html)吃掉,永遠到不了 recover。
    let routes = api
        .recover(handle_rejection)
        .or(ws.recover(handle_rejection))
        .or(serve_client)
        .with(cors);
    maybe_register_body_part(port);
    let _ = Arc::clone(&rooms);

    // BIND (default 0.0.0.0 so LAN devices reach it; Tauri sets 127.0.0.1).
    // HTTPS=1 + certs => serve TLS itself (replaces Vite's HTTPS role; LAN mic needs a secure ctx).
    let ip: std::net::Ipv4Addr = std::env::var("BIND")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(std::net::Ipv4Addr::new(0, 0, 0, 0));
    let cert = std::env::var("TLS_CERT").unwrap_or_else(|_| "certs/cert.pem".into());
    let key = std::env::var("TLS_KEY").unwrap_or_else(|_| "certs/key.pem".into());
    let server = warp::serve(routes);
    if std::env::var("HTTPS").as_deref() == Ok("1")
        && std::path::Path::new(&cert).exists()
        && std::path::Path::new(&key).exists()
    {
        println!("mori-canvas-server (Rust, HTTPS) on https://{ip}:{port}");
        server
            .tls()
            .cert_path(&cert)
            .key_path(&key)
            .run((ip, port))
            .await;
    } else {
        println!("mori-canvas-server (Rust) on http://{ip}:{port}");
        server.run((ip, port)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn llm_local_only_env_values() {
        // "1" / "true"(不分大小寫、容忍空白)視為鎖定,其他都不是
        assert!(parse_local_only_env(Some("1")));
        assert!(parse_local_only_env(Some("true")));
        assert!(parse_local_only_env(Some("TRUE")));
        assert!(parse_local_only_env(Some(" 1 ")));
        assert!(!parse_local_only_env(Some("0")));
        assert!(!parse_local_only_env(Some("false")));
        assert!(!parse_local_only_env(Some("")));
        assert!(!parse_local_only_env(None));
    }

    #[test]
    fn locked_local_only_cannot_be_turned_off() {
        // 鎖定時嘗試關閉 => 值不動、回中文錯誤
        assert_eq!(
            apply_local_only_change(true, true, Some(false)),
            (true, Some("此部署鎖定本機模式"))
        );
        // 鎖定時重複開啟 / 沒帶 localOnly => 照常、無錯誤
        assert_eq!(
            apply_local_only_change(true, true, Some(true)),
            (true, None)
        );
        assert_eq!(apply_local_only_change(true, true, None), (true, None));
        // 未鎖定:自由開關
        assert_eq!(
            apply_local_only_change(false, true, Some(false)),
            (false, None)
        );
        assert_eq!(
            apply_local_only_change(false, false, Some(true)),
            (true, None)
        );
        assert_eq!(apply_local_only_change(false, false, None), (false, None));
    }

    #[test]
    fn settings_access_three_scenarios() {
        use SettingsAccess::*;
        // 設了 token:相符 => 全開(loopback 與否皆然)
        assert_eq!(settings_access(Some("s3cret"), Some("s3cret"), false), Full);
        assert_eq!(settings_access(Some("s3cret"), Some("s3cret"), true), Full);
        // 設了 token:不符 / 沒帶 => 整個請求拒絕,連 loopback 也一樣
        assert_eq!(settings_access(Some("s3cret"), Some("wrong"), true), Denied);
        assert_eq!(settings_access(Some("s3cret"), None, true), Denied);
        assert_eq!(settings_access(Some("s3cret"), None, false), Denied);
        // 未設 token + loopback:維持現狀全開(單機自用)
        assert_eq!(settings_access(None, None, true), Full);
        // 未設 token + 非 loopback:只能改個人偏好;帶了沒用的 token 也一樣
        assert_eq!(settings_access(None, None, false), PersonalOnly);
        assert_eq!(settings_access(None, Some("whatever"), false), PersonalOnly);
    }

    #[test]
    fn settings_access_field_classes() {
        // 三情境 × 主機級/個人欄位
        let admin = settings_access(Some("t"), Some("t"), false);
        assert!(admin.allows_host_fields());
        assert!(admin.allows_personal_fields());
        let denied = settings_access(Some("t"), None, true);
        assert!(!denied.allows_host_fields());
        assert!(!denied.allows_personal_fields());
        let local = settings_access(None, None, true);
        assert!(local.allows_host_fields());
        assert!(local.allows_personal_fields());
        // 未設 token 的公開部署訪客:spacing/autoTidy 可改,whisperUrl/mode/key 等拒改
        let visitor = settings_access(None, None, false);
        assert!(!visitor.allows_host_fields());
        assert!(visitor.allows_personal_fields());
    }

    #[test]
    fn loopback_detection_covers_v4_v6_and_mapped() {
        let p = |s: &str| Some(s.parse::<std::net::SocketAddr>().unwrap());
        assert!(is_loopback_addr(p("127.0.0.1:9999")));
        assert!(is_loopback_addr(p("[::1]:9999")));
        assert!(is_loopback_addr(p("[::ffff:127.0.0.1]:9999")));
        assert!(!is_loopback_addr(p("192.168.1.5:9999")));
        assert!(!is_loopback_addr(p("[2001:db8::1]:9999")));
        // 拿不到來源位址(理論上不會發生)一律當非 loopback,寧可鎖不可放
        assert!(!is_loopback_addr(None));
    }

    #[test]
    fn trusted_local_requires_loopback_and_no_proxy() {
        let p = |s: &str| Some(s.parse::<std::net::SocketAddr>().unwrap());
        // 直連本機、無 proxy = 真管理員
        assert!(is_trusted_local(p("127.0.0.1:9999"), &None));
        assert!(is_trusted_local(p("[::1]:9999"), &Some("".into()))); // 空 XFF 不算 proxy
        // 關鍵:Render/反代用 loopback 連進來,但帶了 XFF → 不是管理員(否則人人 Full 權限)
        assert!(!is_trusted_local(p("127.0.0.1:9999"), &Some("1.2.3.4".into())));
        // 非 loopback 一律不是
        assert!(!is_trusted_local(p("10.0.0.5:9999"), &None));
    }

    #[test]
    fn public_room_list_default_on_demo_off() {
        // 未設 / 其他值 = 公開(自架現狀不變)
        assert!(parse_public_room_list(None));
        assert!(parse_public_room_list(Some("1")));
        assert!(parse_public_room_list(Some("yes")));
        assert!(parse_public_room_list(Some("")));
        // "0" / "false"(含空白、不分大小寫)= 不公開房號
        assert!(!parse_public_room_list(Some("0")));
        assert!(!parse_public_room_list(Some(" 0 ")));
        assert!(!parse_public_room_list(Some("false")));
        assert!(!parse_public_room_list(Some("FALSE")));
    }

    #[test]
    fn demo_board_seed_is_valid_v1_board() {
        // 內嵌種子必須是合法的 mori-canvas/v1 畫板,卡片/圖框齊全 — 編譯期壞檔在這裡擋下
        let v: Value = serde_json::from_str(DEMO_BOARD_JSON).expect("demo board json parses");
        assert_eq!(
            v.get("format").and_then(|x| x.as_str()),
            Some("mori-canvas/v1")
        );
        assert!(!v["shapes"].as_array().unwrap().is_empty());
        assert!(!v["frames"].as_array().unwrap().is_empty());
        assert!(!v["connectors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn client_ip_trusts_last_hop_and_socket_fallback() {
        // XFF 前段可偽造,只信最後一跳
        let xff = Some("99.99.99.99, 5.6.7.8".to_string());
        assert_eq!(client_ip(&xff, &None), "5.6.7.8");
        // 無 proxy 退回 socket 位址 → 自架直連也有真 per-IP 限流
        let addr: Option<std::net::SocketAddr> = Some("9.9.9.9:1234".parse().unwrap());
        assert_eq!(client_ip(&None, &addr), "9.9.9.9");
        assert_eq!(client_ip(&None, &None), "?");
    }

    #[test]
    fn orgchart_export_uses_org_color_meanings() {
        let shapes = vec![
            json!({"id":"ceo","type":"sticky","frameId":"f1","text":"總經理","color":"blue","owner":"亞澤"}),
            json!({"id":"eng","type":"sticky","frameId":"f1","text":"工程部","color":"green"}),
        ];
        let conns = vec![json!({"from":"ceo","to":"eng"})];
        let frames = vec![json!({"id":"f1","type":"orgchart","title":"團隊"})];

        let md = export_markdown_from_parts(&shapes, &conns, &frames, "meeting", "組織", llm::Lang::ZhTw);

        assert!(md.contains("**最高層**"));
        assert!(md.contains("**主管/部門**"));
        assert!(md.contains("**隸屬(上級 → 下屬)**"));
        assert!(!md.contains("**決議**"));
        assert!(!md.contains("**待辦**"));

        // 同一張板用 en 匯出:區段標題、板型名、連線標題全英文,且不留中文
        let md_en =
            export_markdown_from_parts(&shapes, &conns, &frames, "meeting", "組織", llm::Lang::En);
        assert!(md_en.contains("Org Chart"));
        assert!(md_en.contains("**Top level**"));
        assert!(md_en.contains("**Managers / Depts**"));
        assert!(md_en.contains("**Reports to (manager -> report)**"));
        assert!(!md_en.contains("最高層"));
        assert!(!md_en.contains("隸屬"));
    }

    #[test]
    fn summary_input_carries_frame_specific_prompt_context() {
        let shapes =
            vec![json!({"id":"ceo","type":"sticky","frameId":"f1","text":"總經理","color":"blue"})];
        let conns = vec![];
        let frames = vec![json!({"id":"f1","type":"orgchart","title":"團隊"})];

        let (_title, input) =
            summary_board_input_from_parts(&shapes, &conns, &frames, "meeting", "組織");

        assert!(input.contains("組織架構圖"));
        assert!(input.contains("blue=最高層"));
        assert!(input.contains("不要用待辦/風險等會議概念"));
        assert!(input.contains("**最高層**"));
        assert!(!input.contains("[決議]"));
    }
}
