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

/// 常駐示範房:啟動時種入內嵌畫板、每小時重置,TTL 清理永遠豁免(見 lib.rs)。
pub const DEMO_ROOM: &str = "DEMO";

pub struct Room {
    pub awareness: AwarenessRef,
    pub bcast: Arc<BroadcastGroup>,
    /// 目前連著的 websocket 數(分頁數);/api/rooms 的「N 人」就是這個
    pub online: std::sync::atomic::AtomicUsize,
    /// 房內最後活動時間(epoch 秒):doc 有寫入或有人連線就更新,TTL 清理用
    pub last_activity: Arc<std::sync::atomic::AtomicU64>,
    /// 鎖板:true 時只有房主(帶正確 ownerKey 的連線)的寫入會落地
    pub locked: std::sync::atomic::AtomicBool,
    /// 房主鑰匙:建房的第一個 client claim;存 server 端 sidecar,不進共享 doc(否則人人看得到)
    pub owner_key: std::sync::Mutex<Option<String>>,
    _sub: yrs::Subscription,
}

impl Room {
    /// 這條連線的寫入是否落地:唯讀連結一律否;鎖板時只認房主鑰匙。
    pub fn can_write(&self, conn_key: Option<&str>, readonly: bool) -> bool {
        if readonly {
            return false;
        }
        if !self.locked.load(std::sync::atomic::Ordering::Relaxed) {
            return true;
        }
        match (conn_key, self.owner_key.lock().ok().and_then(|g| g.clone())) {
            (Some(k), Some(o)) => k == o,
            _ => false,
        }
    }
}

/// y-protocols 訊息分類:Sync(type 0)的 SyncStep2(1)/Update(2)會改動文件;
/// SyncStep1(0,讀取請求)與 Awareness(type 1,游標/在線)不會。
/// type 與 subtype 都是小值 varint = 單一 byte,直接看前兩個 byte 即可。
pub fn is_doc_write(msg: &[u8]) -> bool {
    msg.first() == Some(&0) && matches!(msg.get(1), Some(1) | Some(2))
}
pub type Rooms = Arc<RwLock<HashMap<String, Arc<Room>>>>;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---- 房間治理(TTL / 上限)純函數:供 cargo test ----

/// 房是否過期:ttl_hours=0 表示永不過期。
pub fn is_room_expired(now_secs: u64, last_activity_secs: u64, ttl_hours: u64) -> bool {
    ttl_hours > 0 && now_secs.saturating_sub(last_activity_secs) > ttl_hours * 3600
}

/// TTL 豁免:DEMO 示範房永遠保留(它有自己的每小時重置)。
pub fn ttl_exempt(name: &str) -> bool {
    name == DEMO_ROOM
}

/// 是否允許再開一間房:已存在的房照常進;max_rooms=0 表示不限。
pub fn allow_new_room(already_exists: bool, current_rooms: usize, max_rooms: usize) -> bool {
    already_exists || max_rooms == 0 || current_rooms < max_rooms
}

/// ROOM_TTL_HOURS env(預設 0 = 不清理)
pub fn room_ttl_hours() -> u64 {
    std::env::var("ROOM_TTL_HOURS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// MAX_ROOMS env(預設 0 = 不限)
fn max_rooms_limit() -> usize {
    std::env::var("MAX_ROOMS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

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

/// 房檔是否已落地(啟動時判斷 DEMO 要不要種子用)
pub fn room_file_exists(name: &str) -> bool {
    room_file(name).exists()
}

/// 取得或建立房。MAX_ROOMS 滿了且這是「全新的房」(記憶體沒有、磁碟也沒檔)就拒絕;
/// 既有的房(載入磁碟快照)照常進。
pub async fn get_or_create_room(rooms: &Rooms, name: &str) -> Result<Arc<Room>, String> {
    if let Some(r) = rooms.read().await.get(name) {
        return Ok(r.clone());
    }
    let mut w = rooms.write().await;
    if let Some(r) = w.get(name) {
        return Ok(r.clone());
    }
    let exists_on_disk = room_file(name).exists();
    if !allow_new_room(exists_on_disk, w.len(), max_rooms_limit()) {
        return Err("房間數已達上限,暫時無法開新房 — 請加入既有房間或稍後再試".into());
    }
    let doc = Doc::new();
    if let Ok(bytes) = std::fs::read(room_file(name)) {
        if let Ok(update) = Update::decode_v1(&bytes) {
            let mut txn = doc.transact_mut();
            let _ = txn.apply_update(update);
        }
    }
    let last_activity = Arc::new(std::sync::atomic::AtomicU64::new(now_secs()));
    let name_owned = name.to_string();
    let la = last_activity.clone();
    let sub = doc
        .observe_update_v1(move |_txn, _e| {
            la.store(now_secs(), std::sync::atomic::Ordering::Relaxed);
            let _ = SAVE_TX.send(name_owned.clone());
        })
        .expect("observe_update_v1");
    let awareness: AwarenessRef = Arc::new(Awareness::new(doc));
    let bcast = Arc::new(BroadcastGroup::new(awareness.clone(), 32).await);
    // 房主/鎖板狀態從 sidecar 載回(server 端持有,不進共享 doc)
    let owner = load_owner_state(name);
    let room = Arc::new(Room {
        awareness,
        bcast,
        online: std::sync::atomic::AtomicUsize::new(0),
        last_activity,
        locked: std::sync::atomic::AtomicBool::new(owner.as_ref().map(|o| o.locked).unwrap_or(false)),
        owner_key: std::sync::Mutex::new(owner.and_then(|o| o.owner_key)),
        _sub: sub,
    });
    w.insert(name.to_string(), room.clone());
    Ok(room)
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct OwnerState {
    pub owner_key: Option<String>,
    pub locked: bool,
}
fn owner_file(name: &str) -> PathBuf {
    room_file(name).with_extension("own.json")
}
fn load_owner_state(name: &str) -> Option<OwnerState> {
    std::fs::read_to_string(owner_file(name))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}
pub fn save_owner_state(name: &str, room: &Room) {
    let st = OwnerState {
        owner_key: room.owner_key.lock().ok().and_then(|g| g.clone()),
        locked: room.locked.load(std::sync::atomic::Ordering::Relaxed),
    };
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(owner_file(name), serde_json::to_string(&st).unwrap_or_default());
}

/// 背景 TTL 清理:每 30 分鐘掃一次。ROOM_TTL_HOURS=0(預設)完全不啟動。
pub fn spawn_room_ttl_cleaner(rooms: Rooms) {
    let ttl = room_ttl_hours();
    if ttl == 0 {
        return;
    }
    println!("room TTL cleaner on: idle rooms expire after {ttl}h (DEMO exempt)");
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
        loop {
            tick.tick().await;
            cleanup_expired_rooms(&rooms, ttl).await;
        }
    });
}

/// 一輪清理:過期且沒人連著的記憶體房 unload + 刪檔;磁碟上沒載入的孤兒檔看 mtime 刪。
pub async fn cleanup_expired_rooms(rooms: &Rooms, ttl_hours: u64) {
    let now = now_secs();
    let expired: Vec<String> = {
        let map = rooms.read().await;
        map.iter()
            .filter(|(name, room)| {
                !ttl_exempt(name)
                    && room.online.load(std::sync::atomic::Ordering::Relaxed) == 0
                    && is_room_expired(
                        now,
                        room.last_activity.load(std::sync::atomic::Ordering::Relaxed),
                        ttl_hours,
                    )
            })
            .map(|(n, _)| n.clone())
            .collect()
    };
    if !expired.is_empty() {
        let mut w = rooms.write().await;
        for name in &expired {
            // 寫鎖下再驗一次:掃描到上鎖之間可能有人剛進房 / 剛寫入
            let still_expired = w
                .get(name)
                .map(|r| {
                    r.online.load(std::sync::atomic::Ordering::Relaxed) == 0
                        && is_room_expired(
                            now,
                            r.last_activity.load(std::sync::atomic::Ordering::Relaxed),
                            ttl_hours,
                        )
                })
                .unwrap_or(false);
            if still_expired {
                w.remove(name);
                let _ = std::fs::remove_file(room_file(name));
                println!("room TTL: expired room {name:?} unloaded + file removed");
            }
        }
    }
    // 沒載入記憶體的舊房檔:用檔案 mtime 判斷
    let loaded: HashSet<PathBuf> = rooms.read().await.keys().map(|n| room_file(n)).collect();
    let demo_file = room_file(DEMO_ROOM);
    if let Ok(rd) = std::fs::read_dir(data_dir()) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) != Some("bin")
                || p == demo_file
                || loaded.contains(&p)
            {
                continue;
            }
            let stale = ent
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|age| age.as_secs() > ttl_hours * 3600)
                .unwrap_or(false);
            if stale {
                let _ = std::fs::remove_file(&p);
                println!("room TTL: stale file {p:?} removed");
            }
        }
    }
}

/// 連線計數的 drop guard:不管是正常關閉、ws 錯誤、還是 task 被 hyper 中途丟掉,
/// guard 一定會 drop → 計數一定會減,不會越加越多。
struct OnlineGuard(Arc<Room>);
impl Drop for OnlineGuard {
    fn drop(&mut self) {
        self.0
            .online
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        // 離線也算活動:TTL 從最後一個人離開起算,不會清到剛散會的房
        self.0
            .last_activity
            .store(now_secs(), std::sync::atomic::Ordering::Relaxed);
    }
}

pub async fn peer(ws: WebSocket, room: Arc<Room>, conn_key: Option<String>, readonly: bool) {
    room.online
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    room.last_activity
        .store(now_secs(), std::sync::atomic::Ordering::Relaxed);
    let _guard = OnlineGuard(room.clone());
    let (sink, stream) = ws.split();
    let sink = Arc::new(Mutex::new(WarpSink::from(sink)));
    let stream = WarpStream::from(stream);
    // server-side enforce:沒有寫入權的連線,文件寫入訊息在這裡被丟棄 —
    // 不是 UI 隱藏;游標/在線(awareness)與讀取(SyncStep1)照常通過。
    let perm_room = room.clone();
    let stream = stream.filter(move |item| {
        let pass = match item {
            Ok(bytes) => !is_doc_write(bytes) || perm_room.can_write(conn_key.as_deref(), readonly),
            Err(_) => true,
        };
        futures_util::future::ready(pass)
    });
    let sub = room.bcast.subscribe(sink, stream);
    let _ = sub.completed().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_write_classification() {
        // Sync(0) + SyncStep1(0) = 讀取請求,放行
        assert!(!is_doc_write(&[0, 0, 1, 2, 3]));
        // Sync(0) + SyncStep2(1) / Update(2) = 文件寫入,要被權限管
        assert!(is_doc_write(&[0, 1, 9]));
        assert!(is_doc_write(&[0, 2, 9]));
        // Awareness(1) = 游標/在線,放行
        assert!(!is_doc_write(&[1, 5, 5]));
        assert!(!is_doc_write(&[]));
    }

    #[test]
    fn room_expiry_respects_ttl_and_zero_means_forever() {
        // ttl=0 => 永不過期
        assert!(!is_room_expired(1_000_000, 0, 0));
        // 剛好在 TTL 內(含邊界)不過期
        assert!(!is_room_expired(72 * 3600, 0, 72));
        assert!(!is_room_expired(1000, 500, 1));
        // 超過 TTL 過期
        assert!(is_room_expired(72 * 3600 + 1, 0, 72));
        assert!(is_room_expired(10 * 3600, 0, 2));
        // last_activity 在未來(時鐘漂移)不可 panic、不過期
        assert!(!is_room_expired(100, 200, 1));
    }

    #[test]
    fn demo_room_is_ttl_exempt() {
        assert!(ttl_exempt("DEMO"));
        assert!(!ttl_exempt("demo")); // 房名區分大小寫,只有正字 DEMO 豁免
        assert!(!ttl_exempt("ABCD"));
    }

    #[test]
    fn max_rooms_blocks_only_brand_new_rooms() {
        // 0 = 不限
        assert!(allow_new_room(false, 9999, 0));
        // 未達上限可開新房
        assert!(allow_new_room(false, 4, 5));
        // 滿了:新房被拒
        assert!(!allow_new_room(false, 5, 5));
        assert!(!allow_new_room(false, 6, 5));
        // 滿了:已存在的房(磁碟有檔 / 記憶體已載)照常進
        assert!(allow_new_room(true, 5, 5));
    }
}
