//! Port of llm.ts — Groq (gpt-oss-120b) -> Ollama (qwen3) cascade via reqwest.
//! Key + models from the shared ~/.mori/config.json.
use serde_json::{json, Value};

pub struct Msg {
    pub role: &'static str,
    pub content: String,
}

/// AI 輸出語言(來自 client 的 X-Lang header 或 ?lang= query)。
/// 預設 ZhTw —— 沒帶 header 的請求行為與改版前完全相同(零回歸)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    ZhTw,
    En,
}
impl Lang {
    /// "en" / "en-US" / "EN"(前綴比對、不分大小寫)=> En;其他(含 None / "zh-TW")=> ZhTw。
    /// 注意不能用 byte slice 取前綴 —— header 值可能是任意 UTF-8,切在 char 邊界外會 panic。
    pub fn parse(v: Option<&str>) -> Lang {
        match v.map(|s| s.trim().to_ascii_lowercase()) {
            Some(s) if s == "en" || s.starts_with("en-") => Lang::En,
            _ => Lang::ZhTw,
        }
    }
}

/// lang=en 時附加在「生成型」system prompt(board-agent / card-edit / summary)尾端的
/// 英文輸出指令。prompts/*.md 本體保持 zh 預設,不動檔案。
pub const EN_OUTPUT_DIRECTIVE: &str = "\n\n[LANGUAGE OVERRIDE — HIGHEST PRIORITY] Ignore every Traditional-Chinese instruction above. EVERY output string MUST be in natural, concise English: every sticky \"text\", every frame \"title\", every tag, owner label and summary line. A Chinese sticky text is WRONG output. The surrounding instructions are in Chinese only because that is the default UI language — your output language is English. Keep proper nouns and people's names as they appear in the source.";
/// lang=en 時也注入到 user message 開頭 —— system 尾端的指令容易被前面大量中文指示稀釋,
/// user 訊息對「輸出語言」的權重更高,雙管齊下才壓得住 gpt-oss 的中文預設。
pub const EN_USER_PREFIX: &str = "[Reply in English. All card text and titles must be English.]\n\n";
/// 清稿(stage-1)是「整理逐字稿」不是生成:en 時只放寬語言規則、明確禁止翻譯,
/// 逐字稿保留講者原語言(逐字記錄的忠實度;翻譯交給後面的 board-agent 輸出層)。
pub const EN_CLEANUP_DIRECTIVE: &str = "\n\n[Language override] The transcript may be in English. Apply the same cleanup rules and reply in the SAME language the speaker used — do not translate. This overrides the Traditional Chinese rule above.";

/// 生成型 prompt 的語言組裝(純函數供測試):zh 原樣;en 附加英文輸出指令。
pub fn with_output_lang(system: String, lang: Lang) -> String {
    match lang {
        Lang::ZhTw => system,
        Lang::En => system + EN_OUTPUT_DIRECTIVE,
    }
}
/// user message 的語言前綴:zh 原樣;en 在最前面壓一行英文輸出指令。
pub fn with_user_lang(user: String, lang: Lang) -> String {
    match lang {
        Lang::ZhTw => user,
        Lang::En => format!("{}{}", EN_USER_PREFIX, user),
    }
}
/// 清稿 prompt 的語言組裝(純函數供測試):zh 原樣;en 附加「同語言、不翻譯」指令。
pub fn with_cleanup_lang(system: String, lang: Lang) -> String {
    match lang {
        Lang::ZhTw => system,
        Lang::En => system + EN_CLEANUP_DIRECTIVE,
    }
}

/// optional visitor-supplied "bring your own AI" — any OpenAI-compatible endpoint
/// (OpenAI / Groq / Azure / Gemini-compat / OpenRouter / Ollama). When base+key+model
/// are all set, the request uses these instead of the host's Groq, so the visitor pays.
/// `lang` rides along so every LLM stage sees the requested output language.
#[derive(Default, Clone)]
pub struct LlmOpts {
    pub base: Option<String>,
    pub key: Option<String>,
    pub model: Option<String>,
    pub lang: Lang,
}
impl LlmOpts {
    fn custom(&self) -> Option<(&str, &str, &str)> {
        let b = self
            .base
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let k = self
            .key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let m = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        Some((b, k, m))
    }
}

async fn call_custom(
    base: &str,
    key: &str,
    model: &str,
    messages: &[Msg],
    json_mode: bool,
) -> Result<String, String> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let mut body = json!({ "model": model, "messages": msgs });
    if json_mode {
        body["response_format"] = json!({ "type": "json_object" });
    }
    let res = reqwest::Client::new()
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("自訂 AI 連不到 {url}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "自訂 AI {}: {}",
            res.status(),
            res.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect::<String>()
        ));
    }
    let data: Value = res
        .json()
        .await
        .map_err(|e| format!("自訂 AI 回應非 JSON: {e}"))?;
    data.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "自訂 AI 沒有回 content".into())
}

fn mori_config() -> Value {
    let home = std::env::var("HOME").unwrap_or_default();
    std::fs::read_to_string(format!("{}/.mori/config.json", home))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}))
}

fn is_placeholder(k: &str) -> bool {
    k.starts_with("REPLACE") || k.contains("YOUR_GROQ") || k == "TODO"
}

// a Groq key the user pastes in the settings UI at runtime (for a machine with no env /
// ~/.mori key and no mori-ear). Session-only; env still wins so a public deploy can't be
// overridden by a visitor. Powers both the AI (Groq) and cloud STT (Groq Whisper).
static RUNTIME_GROQ_KEY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
pub fn set_runtime_groq_key(k: &str) {
    let k = k.trim();
    let v = if k.is_empty() || is_placeholder(k) {
        None
    } else {
        Some(k.to_string())
    };
    if let Ok(mut g) = RUNTIME_GROQ_KEY.lock() {
        *g = v;
    }
}

pub fn groq_key() -> Option<String> {
    if let Ok(env) = std::env::var("GROQ_API_KEY") {
        if !env.is_empty() && !is_placeholder(&env) {
            return Some(env);
        }
    }
    if let Some(k) = RUNTIME_GROQ_KEY.lock().ok().and_then(|g| g.clone()) {
        return Some(k);
    }
    let c = mori_config();
    c.get("providers")
        .and_then(|p| p.get("groq"))
        .and_then(|g| g.get("api_key"))
        .and_then(|k| k.as_str())
        .filter(|k| !is_placeholder(k))
        .map(|s| s.to_string())
}

pub fn config_info() -> Value {
    let c = mori_config();
    let g = |path: &[&str], d: &str| -> String {
        let mut cur = &c;
        for p in path {
            match cur.get(p) {
                Some(v) => cur = v,
                None => return d.to_string(),
            }
        }
        cur.as_str().unwrap_or(d).to_string()
    };
    json!({
        "llmGroqModel": g(&["providers","groq","model"], "openai/gpt-oss-120b"),
        "llmOllamaModel": g(&["providers","ollama","model"], "qwen3:4b-instruct-2507-q4_K_M"),
        "sttProvider": g(&["stt_provider"], "groq"),
        "sttGroqModel": g(&["providers","groq","stt_model"], "whisper-large-v3-turbo"),
        "sttLocalModel": g(&["providers","whisper-local","model_path"], "(未設定)"),
    })
}

async fn call_groq(messages: &[Msg], json_mode: bool) -> Result<String, String> {
    let key = groq_key().ok_or("no groq api key")?;
    let c = mori_config();
    let model = c
        .get("providers")
        .and_then(|p| p.get("groq"))
        .and_then(|g| g.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("openai/gpt-oss-120b")
        .to_string();
    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let mut body = json!({ "model": model, "messages": msgs });
    if json_mode {
        body["response_format"] = json!({ "type": "json_object" });
    }
    let res = reqwest::Client::new()
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("groq req: {e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "groq {}: {}",
            res.status(),
            res.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    let data: Value = res.json().await.map_err(|e| format!("groq json: {e}"))?;
    data.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "groq: no content".into())
}

async fn call_ollama(messages: &[Msg], json_mode: bool) -> Result<String, String> {
    let c = mori_config();
    let base = c
        .get("providers")
        .and_then(|p| p.get("ollama"))
        .and_then(|o| o.get("base_url"))
        .and_then(|b| b.as_str())
        .unwrap_or("http://localhost:11434")
        .trim_end_matches('/')
        .to_string();
    let model = c
        .get("providers")
        .and_then(|p| p.get("ollama"))
        .and_then(|o| o.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("qwen3:4b-instruct-2507-q4_K_M")
        .to_string();
    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let mut body = json!({ "model": model, "messages": msgs, "stream": false, "think": false, "options": { "num_ctx": 8192 } });
    if json_mode {
        body["format"] = json!("json");
    }
    let res = reqwest::Client::new()
        .post(format!("{}/api/chat", base))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ollama unreachable at {base}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("ollama {}", res.status()));
    }
    let data: Value = res.json().await.map_err(|e| format!("ollama json: {e}"))?;
    data.get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "ollama: no content".into())
}

/// Hard-convert any model output to Taiwan Traditional Chinese. LLMs (gpt-oss / qwen)
/// follow a "use 繁體 not 簡體" instruction unreliably, so we don't trust them — we convert
/// deterministically with ferrous-opencc S2twp (OpenCC official dict + 台灣詞彙片語, same
/// as mori-meeting-recorder). Pure Rust, dict bundled => no external install / no C dep.
/// JSON keys are ASCII so structure is untouched; only Chinese values flip. No-op on failure.
pub fn to_traditional(text: &str) -> String {
    use ferrous_opencc::config::BuiltinConfig;
    use ferrous_opencc::OpenCC;
    static CC: std::sync::OnceLock<Option<OpenCC>> = std::sync::OnceLock::new();
    match CC
        .get_or_init(|| OpenCC::from_config(BuiltinConfig::S2twp).ok())
        .as_ref()
    {
        Some(cc) => cc.convert(text),
        None => text.to_string(),
    }
}

/// 輸出後處理(純函數供測試):zh 過 OpenCC 繁化(LLM 不可信任,硬轉);
/// en 跳過 —— 英文沒有簡繁問題,過 opencc 雖無害但語意上不該過。
pub fn localize_output(text: String, lang: Lang) -> String {
    match lang {
        Lang::ZhTw => to_traditional(&text),
        Lang::En => text,
    }
}

/// Visitor's own AI (if configured) > Groq > Ollama. local_only => Ollama only.
/// zh output always passes through to_traditional() so cards/summaries are never 簡體;
/// lang=en skips the conversion (see localize_output).
pub async fn chat(
    messages: &[Msg],
    json_mode: bool,
    local_only: bool,
    opts: &LlmOpts,
) -> Result<(String, String), String> {
    let (text, provider) = chat_raw(messages, json_mode, local_only, opts).await?;
    Ok((localize_output(text, opts.lang), provider))
}

/// chat_raw 的路由決策(抽成純函數供測試)。重點:本機模式優先於 BYO ——
/// 訪客自帶的 X-LLM-Base 端點多半在雲端,若先看 BYO 就成了 local_only 的繞道,
/// 所以 local_only 時 BYO 一律忽略、只走本機 Ollama。
#[derive(Debug, PartialEq)]
enum Route {
    /// 訪客自帶端點(僅非本機模式)
    Byo,
    /// 本機模式:只走 Ollama;byo_ignored 標示有帶 BYO 但被忽略
    LocalOnly { byo_ignored: bool },
    /// 預設串接:Groq -> Ollama
    Cascade,
}
fn pick_route(local_only: bool, has_byo: bool) -> Route {
    if local_only {
        Route::LocalOnly {
            byo_ignored: has_byo,
        }
    } else if has_byo {
        Route::Byo
    } else {
        Route::Cascade
    }
}

// ── demo 共用 key 的每日 AI 上限:保護站長的 Groq 預算。只擋 Cascade(用站長共用 key);
// 訪客自備 key(Byo)與本機模式(LocalOnly)完全不受限。DEMO_AI_DAILY_LIMIT 未設或 0 = 不限。
fn demo_ai_daily_limit() -> Option<u64> {
    std::env::var("DEMO_AI_DAILY_LIMIT")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
}
/// 純函數供測試:今天已用 count 次、上限 limit,還能不能再呼叫一次共用 key。
fn within_daily_cap(count: u64, limit: Option<u64>) -> bool {
    match limit {
        None => true,
        Some(l) => count < l,
    }
}
static AI_DAY: std::sync::Mutex<(u64, u64)> = std::sync::Mutex::new((0, 0)); // (utc_day, count)
fn utc_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86400)
        .unwrap_or(0)
}
/// 記一次共用 key 呼叫;回 true=放行(已計入今日),false=今日額度已滿。跨 UTC 日界線自動歸零。
fn record_shared_key_call() -> bool {
    let limit = demo_ai_daily_limit();
    let mut g = AI_DAY.lock().unwrap_or_else(|e| e.into_inner());
    let today = utc_day();
    if g.0 != today {
        *g = (today, 0);
    }
    if !within_daily_cap(g.1, limit) {
        return false;
    }
    g.1 += 1;
    true
}
/// 額度用完時回給使用者的友善訊息(端點直接顯示在 error 欄)。
const AI_CAP_MSG: &str =
    "今天的試用 AI 額度已用完。想繼續用,在設定填自己的免費 Groq key(console.groq.com),或自行部署一份(全開源)。";

async fn chat_raw(
    messages: &[Msg],
    json_mode: bool,
    local_only: bool,
    opts: &LlmOpts,
) -> Result<(String, String), String> {
    match pick_route(local_only, opts.custom().is_some()) {
        Route::LocalOnly { byo_ignored } => {
            // provider 字串標明 BYO 被本機模式忽略,讓呼叫端/前端看得出不是走訪客端點
            let provider = if byo_ignored {
                "ollama(local-only,byo-ignored)"
            } else {
                "ollama(local-only)"
            };
            Ok((call_ollama(messages, json_mode).await?, provider.into()))
        }
        Route::Byo => {
            let (base, key, model) = opts.custom().expect("pick_route 已確認有 BYO");
            Ok((
                call_custom(base, key, model, messages, json_mode).await?,
                format!("byo:{}", model),
            ))
        }
        Route::Cascade => {
            // 共用 key 每日額度:滿了直接擋,連 Groq/Ollama 都不打(不花站長的錢),回友善訊息
            if !record_shared_key_call() {
                return Err(AI_CAP_MSG.to_string());
            }
            match call_groq(messages, json_mode).await {
                Ok(t) => Ok((t, "groq:gpt-oss-120b".into())),
                Err(ge) => match call_ollama(messages, json_mode).await {
                    Ok(t) => Ok((t, "ollama".into())),
                    Err(oe) => Err(format!(
                        "both LLM providers failed — groq: {ge}; ollama: {oe}"
                    )),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_only_takes_priority_over_byo() {
        use super::{pick_route, Route};
        // 本機模式下帶 X-LLM-Base 也不准繞去訪客端點(可能在雲端)
        assert_eq!(
            pick_route(true, true),
            Route::LocalOnly { byo_ignored: true }
        );
        assert_eq!(
            pick_route(true, false),
            Route::LocalOnly { byo_ignored: false }
        );
        // 非本機模式:BYO 照常優先,沒 BYO 走 Groq -> Ollama 串接
        assert_eq!(pick_route(false, true), Route::Byo);
        assert_eq!(pick_route(false, false), Route::Cascade);
    }

    #[test]
    fn daily_cap_allows_until_limit_then_blocks() {
        use super::within_daily_cap;
        // 未設上限(None)=> 永遠放行
        assert!(within_daily_cap(0, None));
        assert!(within_daily_cap(999_999, None));
        // 上限 100:0..99 放行,100 起擋(count 是「已用」次數)
        assert!(within_daily_cap(0, Some(100)));
        assert!(within_daily_cap(99, Some(100)));
        assert!(!within_daily_cap(100, Some(100)));
        assert!(!within_daily_cap(101, Some(100)));
    }

    #[test]
    fn lang_parse_header_values() {
        use super::Lang;
        // en 系列(含 region tag、大小寫、空白)=> En
        assert_eq!(Lang::parse(Some("en")), Lang::En);
        assert_eq!(Lang::parse(Some("EN")), Lang::En);
        assert_eq!(Lang::parse(Some("en-US")), Lang::En);
        assert_eq!(Lang::parse(Some(" en ")), Lang::En);
        // 預設與 zh 系列 => ZhTw(沒帶 header 行為不變)
        assert_eq!(Lang::parse(None), Lang::ZhTw);
        assert_eq!(Lang::parse(Some("zh-TW")), Lang::ZhTw);
        assert_eq!(Lang::parse(Some("zh")), Lang::ZhTw);
        assert_eq!(Lang::parse(Some("")), Lang::ZhTw);
        assert_eq!(Lang::parse(Some("english")), Lang::ZhTw); // 不認模糊值,寧可回預設
        assert_eq!(Lang::parse(Some("enx")), Lang::ZhTw);
        assert_eq!(Lang::parse(Some("中文標頭")), Lang::ZhTw); // 任意 UTF-8 不可 panic
    }

    #[test]
    fn prompt_assembly_appends_directive_only_for_en() {
        use super::{with_cleanup_lang, with_output_lang, Lang};
        let sys = "你是白板整理助手。".to_string();
        // zh:prompt 原樣 —— 零回歸
        assert_eq!(with_output_lang(sys.clone(), Lang::ZhTw), sys);
        assert_eq!(with_cleanup_lang(sys.clone(), Lang::ZhTw), sys);
        // en:生成型 prompt 附加英文輸出指令(在尾端、保留原 prompt)
        let out = with_output_lang(sys.clone(), Lang::En);
        assert!(out.starts_with(&sys));
        assert!(out.contains("in natural, concise English"));
        // en:清稿 prompt 附加「同語言、不翻譯」指令(不是輸出英文指令)
        let cl = with_cleanup_lang(sys.clone(), Lang::En);
        assert!(cl.starts_with(&sys));
        assert!(cl.contains("do not translate"));
        assert!(!cl.contains("in natural, concise English"));
    }

    #[test]
    fn localize_output_skips_opencc_for_en() {
        use super::{localize_output, Lang};
        // zh:簡體硬轉繁體(原行為)
        assert_eq!(localize_output("开始节点".into(), Lang::ZhTw), "開始節點");
        // en:原樣通過 —— 不過 opencc(就算內容夾雜簡體也不動,輸出語言由 prompt 管)
        assert_eq!(localize_output("Start node".into(), Lang::En), "Start node");
        assert_eq!(localize_output("开始节点".into(), Lang::En), "开始节点");
    }

    #[test]
    fn converts_simplified_to_traditional() {
        // the exact failure the user saw: 开始节点 must become 開始節點
        assert_eq!(super::to_traditional("开始节点"), "開始節點");
        assert_eq!(super::to_traditional("软件"), "軟體"); // 台灣詞彙(s2twp,非僅字形)
                                                           // JSON keys (ASCII) stay intact; the Chinese value flips to 繁體
        let j = super::to_traditional(r#"{"text":"开始决定"}"#);
        assert!(j.contains("\"text\""), "JSON key kept");
        assert!(j.contains("開始"), "value converted: {j}");
        assert!(!j.contains("开"), "no simplified left: {j}");
    }
}
