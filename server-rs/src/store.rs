//! Read/write helpers over a room's yrs maps (shapes / connectors / frames / meta),
//! bridging to serde_json so the rest of the server works in plain JSON.
use crate::sync::Room;
use crate::yval::{any_to_json, json_to_any, map_values_json};
use serde_json::{json, Value};
use yrs::types::ToJson;
use yrs::{Map, Transact};

pub fn read_map(room: &Room, name: &str) -> Vec<Value> {
    let doc = room.awareness.doc();
    let map = doc.get_or_insert_map(name);
    let txn = doc.transact();
    map_values_json(&txn, &map)
}

pub fn read_meta(room: &Room) -> (String, String) {
    let doc = room.awareness.doc();
    let meta = doc.get_or_insert_map("meta");
    let txn = doc.transact();
    let get = |k: &str| -> Option<String> {
        meta.get(&txn, k)
            .and_then(|o| match any_to_json(&o.to_json(&txn)) {
                Value::String(s) => Some(s),
                _ => None,
            })
    };
    (
        get("type").unwrap_or_else(|| "meeting".into()),
        get("topic").unwrap_or_default(),
    )
}

pub fn set_meta(room: &Room, typ: Option<&str>, topic: Option<&str>) {
    let doc = room.awareness.doc();
    let meta = doc.get_or_insert_map("meta");
    let mut txn = doc.transact_mut();
    if let Some(t) = typ {
        meta.insert(&mut txn, "type", t.to_string());
    }
    if let Some(t) = topic {
        meta.insert(&mut txn, "topic", t.chars().take(80).collect::<String>());
    }
}

/// apply tidy positions to shapes + placements (x/y/w/h) to frames in one transaction
pub fn apply_tidy(
    room: &Room,
    positions: &[(String, f64, f64)],
    frames_out: &[crate::layout::FramePlace],
) {
    let doc = room.awareness.doc();
    let shapes = doc.get_or_insert_map("shapes");
    let frames = doc.get_or_insert_map("frames");
    let mut txn = doc.transact_mut();
    for (id, x, y) in positions {
        if let Some(cur) = shapes.get(&txn, id) {
            let mut v = any_to_json(&cur.to_json(&txn));
            v["x"] = json!(x);
            v["y"] = json!(y);
            shapes.insert(&mut txn, id.clone(), json_to_any(&v));
        }
    }
    for f in frames_out {
        if let Some(cur) = frames.get(&txn, &f.id) {
            let mut v = any_to_json(&cur.to_json(&txn));
            v["x"] = json!(f.x);
            v["y"] = json!(f.y);
            v["w"] = json!(f.w);
            v["h"] = json!(f.h);
            frames.insert(&mut txn, f.id.clone(), json_to_any(&v));
        }
    }
}

/// last `n` lines of the room's shared transcript log ("name:text"), oldest→newest —
/// fed to the agent as discussion context so cards reflect the whole conversation.
pub fn read_transcript_tail(room: &Room, n: usize) -> Vec<String> {
    use yrs::types::ToJson;
    use yrs::Array;
    let doc = room.awareness.doc();
    let arr = doc.get_or_insert_array("transcript");
    let txn = doc.transact();
    let all: Vec<Value> = arr
        .iter(&txn)
        .map(|v| any_to_json(&v.to_json(&txn)))
        .collect();
    all.iter()
        .rev()
        .take(n)
        .rev()
        .filter_map(|j| {
            let text = j.get("text").and_then(|x| x.as_str()).unwrap_or("").trim();
            if text.is_empty() {
                return None;
            }
            let by = j.get("by").and_then(|x| x.as_str()).unwrap_or("");
            Some(if by.is_empty() {
                text.to_string()
            } else {
                format!("{}:{}", by, text)
            })
        })
        .collect()
}

pub fn frames_sorted(room: &Room) -> Vec<Value> {
    let mut f = read_map(room, "frames");
    f.sort_by(|a, b| {
        a.get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .cmp(b.get("id").and_then(|x| x.as_str()).unwrap_or(""))
    });
    f
}

/// create a new frame to the right of existing ones; returns the frame Value
pub fn create_frame(room: &Room, typ: &str, title: &str) -> Value {
    let list = read_map(room, "frames");
    let (mut x, mut y) = (80.0_f64, 80.0_f64);
    if let Some(right) = list.iter().max_by(|a, b| {
        let ra = a.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0)
            + a.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let rb = b.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0)
            + b.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0);
        ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
    }) {
        x = right.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0)
            + right.get("w").and_then(|v| v.as_f64()).unwrap_or(480.0)
            + 90.0;
        y = right.get("y").and_then(|v| v.as_f64()).unwrap_or(80.0);
    }
    let id = format!("frame-{}", rid());
    let label = if title.is_empty() {
        crate::board_types::board_type(typ).label
    } else {
        title
    };
    let f =
        json!({ "id": id, "title": label, "type": typ, "x": x, "y": y, "w": 480.0, "h": 320.0 });
    let doc = room.awareness.doc();
    let frames = doc.get_or_insert_map("frames");
    let mut txn = doc.transact_mut();
    frames.insert(&mut txn, id.clone(), json_to_any(&f));
    f
}

/// 把一份 mori-canvas/v1 畫板 JSON 整批寫進房(同 client 的 applyBoardData):
/// 先清空 shapes / connectors / frames / transcript,再依檔案內容重建 + 設 meta。
/// DEMO 示範房的種子與每小時重置都走這裡。
pub fn seed_board(room: &Room, data: &Value) {
    use yrs::Array;
    let doc = room.awareness.doc();
    let shapes = doc.get_or_insert_map("shapes");
    let conns = doc.get_or_insert_map("connectors");
    let frames = doc.get_or_insert_map("frames");
    let meta = doc.get_or_insert_map("meta");
    let transcript = doc.get_or_insert_array("transcript");
    let mut txn = doc.transact_mut();
    shapes.clear(&mut txn);
    conns.clear(&mut txn);
    frames.clear(&mut txn);
    let tlen = transcript.len(&txn);
    if tlen > 0 {
        transcript.remove_range(&mut txn, 0, tlen);
    }
    let insert_all = |txn: &mut yrs::TransactionMut, map: &yrs::MapRef, key: &str| {
        for item in data.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                map.insert(txn, id.to_string(), json_to_any(item));
            }
        }
    };
    insert_all(&mut txn, &frames, "frames");
    insert_all(&mut txn, &shapes, "shapes");
    insert_all(&mut txn, &conns, "connectors");
    for line in data
        .get("transcript")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        transcript.push_back(&mut txn, json_to_any(line));
    }
    if let Some(t) = data.pointer("/meta/type").and_then(|v| v.as_str()) {
        meta.insert(&mut txn, "type", t.to_string());
    }
    if let Some(t) = data.pointer("/meta/topic").and_then(|v| v.as_str()) {
        meta.insert(&mut txn, "topic", t.to_string());
    }
}

/// clear all shapes + connectors + frames (room end / clear)
pub fn clear_room(room: &Room) {
    let doc = room.awareness.doc();
    let shapes = doc.get_or_insert_map("shapes");
    let conns = doc.get_or_insert_map("connectors");
    let frames = doc.get_or_insert_map("frames");
    let mut txn = doc.transact_mut();
    shapes.clear(&mut txn);
    conns.clear(&mut txn);
    frames.clear(&mut txn);
}

pub fn rid() -> String {
    // crude unique id (no rand crate): nanos + counter
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{:x}", n, C.fetch_add(1, Ordering::Relaxed))
}
