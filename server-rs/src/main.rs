mod sync;
mod yval;

use warp::Filter;

#[tokio::main]
async fn main() {
    let rooms = sync::new_rooms();
    sync::init_persistence(rooms.clone());

    let health = warp::get()
        .and(warp::path!("api" / "health"))
        .map(|| warp::reply::json(&serde_json::json!({ "ok": true, "server": "rust" })));

    let rooms_ws = rooms.clone();
    let ws = warp::path::tail().and(warp::ws()).and_then(move |tail: warp::path::Tail, ws: warp::ws::Ws| {
        let rooms = rooms_ws.clone();
        async move {
            let mut name = tail.as_str().to_string();
            if let Some(r) = name.strip_prefix("sync/") {
                name = r.to_string();
            }
            let name = percent_encoding::percent_decode_str(&name).decode_utf8_lossy().to_string();
            let room = sync::get_or_create_room(&rooms, &name).await;
            Ok::<_, warp::Rejection>(ws.on_upgrade(move |socket| sync::peer(socket, room)))
        }
    });

    let routes = health.or(ws);
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(1334);
    println!("mori-canvas-server (Rust) on http://127.0.0.1:{port}");
    warp::serve(routes).run(([127, 0, 0, 1], port)).await;
}
