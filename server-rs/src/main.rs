// 同 lib.rs:warp filter 鏈的巨型嵌套型別需要更高的遞迴上限(monomorphize 發生在 bin 這層)
#![recursion_limit = "256"]
#[tokio::main]
async fn main() {
    // load a local .env if present (gitignored) — for self-host / demo config without
    // exporting vars by hand. Render/cloud use real dashboard env vars (no .env there).
    let _ = dotenvy::dotenv();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(1334);
    mori_canvas_server::serve(port).await;
}
