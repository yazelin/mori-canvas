//! Multi-room yrs sync (y-websocket protocol, interops with the yjs JS client) +
//! debounced per-room snapshot persistence to .data/<room>.bin.
use futures_util::StreamExt;
use once_cell::sync::{Lazy, OnceCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use warp::ws::WebSocket;
use yrs::sync::Awareness;
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};
use yrs_warp::broadcast::BroadcastGroup;
use yrs_warp::ws::{WarpSink, WarpStream};
use yrs_warp::AwarenessRef;

pub struct Room {
    pub awareness: AwarenessRef,
    pub bcast: Arc<BroadcastGroup>,
    /// 目前連著的 websocket 數(分頁數);/api/rooms 的「N 人」就是這個
    pub online: std::sync::atomic::AtomicUsize,
    _sub: yrs::Subscription,
}
pub type Rooms = Arc<RwLock<HashMap<String, Arc<Room>>>>;

pub fn new_rooms() -> Rooms {
    Arc::new(RwLock::new(HashMap::new()))
}

fn data_dir() -> PathBuf {
    PathBuf::from(".data")
}
fn room_file(name: &str) -> PathBuf {
    let enc: String =
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();
    let base = if enc.len() > 120 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut h);
        format!("{}-{:x}", &enc[..100], h.finish())
    } else if enc.is_empty() {
        "default".to_string()
    } else {
        enc
    };
    data_dir().join(format!("{}.bin", base))
}

static ROOMS_FOR_SAVE: OnceCell<Rooms> = OnceCell::new();
static SAVE_TX: Lazy<mpsc::UnboundedSender<String>> = Lazy::new(|| {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut dirty: HashSet<String> = HashSet::new();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(600));
        loop {
            tokio::select! {
                Some(name) = rx.recv() => { dirty.insert(name); }
                _ = tick.tick() => {
                    if dirty.is_empty() { continue; }
                    if let Some(rooms) = ROOMS_FOR_SAVE.get() {
                        let map = rooms.read().await;
                        for name in dirty.drain() {
                            if let Some(room) = map.get(&name) { save_room(&name, room).await; }
                        }
                    }
                }
            }
        }
    });
    tx
});

pub fn init_persistence(rooms: Rooms) {
    std::fs::create_dir_all(data_dir()).ok();
    let _ = ROOMS_FOR_SAVE.set(rooms);
    Lazy::force(&SAVE_TX);
}

async fn save_room(name: &str, room: &Room) {
    let bytes = {
        let txn = room.awareness.doc().transact();
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    let _ = tokio::fs::write(room_file(name), bytes).await;
}

pub async fn get_or_create_room(rooms: &Rooms, name: &str) -> Arc<Room> {
    if let Some(r) = rooms.read().await.get(name) {
        return r.clone();
    }
    let mut w = rooms.write().await;
    if let Some(r) = w.get(name) {
        return r.clone();
    }
    let doc = Doc::new();
    if let Ok(bytes) = std::fs::read(room_file(name)) {
        if let Ok(update) = Update::decode_v1(&bytes) {
            let mut txn = doc.transact_mut();
            let _ = txn.apply_update(update);
        }
    }
    let name_owned = name.to_string();
    let sub = doc
        .observe_update_v1(move |_txn, _e| {
            let _ = SAVE_TX.send(name_owned.clone());
        })
        .expect("observe_update_v1");
    let awareness: AwarenessRef = Arc::new(Awareness::new(doc));
    let bcast = Arc::new(BroadcastGroup::new(awareness.clone(), 32).await);
    let room = Arc::new(Room {
        awareness,
        bcast,
        online: std::sync::atomic::AtomicUsize::new(0),
        _sub: sub,
    });
    w.insert(name.to_string(), room.clone());
    room
}

/// 連線計數的 drop guard:不管是正常關閉、ws 錯誤、還是 task 被 hyper 中途丟掉,
/// guard 一定會 drop → 計數一定會減,不會越加越多。
struct OnlineGuard(Arc<Room>);
impl Drop for OnlineGuard {
    fn drop(&mut self) {
        self.0
            .online
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

pub async fn peer(ws: WebSocket, room: Arc<Room>) {
    room.online
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let _guard = OnlineGuard(room.clone());
    let (sink, stream) = ws.split();
    let sink = Arc::new(Mutex::new(WarpSink::from(sink)));
    let stream = WarpStream::from(stream);
    let sub = room.bcast.subscribe(sink, stream);
    let _ = sub.completed().await;
}
