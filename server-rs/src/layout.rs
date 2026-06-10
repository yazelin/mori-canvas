//! Port of the layout positioners + frame-aware tidy from sync-server.ts.
//! Operates on plain data (cards/connectors/frames); returns id->(x,y) positions
//! and per-frame sizes, which the caller applies in a yrs transaction.
use crate::board_types::board_type;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const CARD_W: f64 = 200.0;
pub const CARD_H: f64 = 200.0;
const COL_GAP: f64 = 50.0;
const ROW_GAP: f64 = 36.0;
pub const X0: f64 = 120.0;
pub const Y0: f64 = 120.0;
const FRAME_PAD: f64 = 28.0;
const FRAME_HEAD: f64 = 60.0;
const COL_ORDER: [&str; 4] = ["yellow", "green", "blue", "red"];

#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub color: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub frame_id: Option<String>,
    pub owner: Option<String>,
}

fn numf(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(d)
}
pub fn card_from(v: &Value) -> Option<Card> {
    if v.get("type").and_then(|t| t.as_str()) != Some("sticky") {
        return None;
    }
    // 備註 are free annotations, not diagram nodes — auto-arrange leaves them where they are
    if v.get("note").and_then(|n| n.as_bool()) == Some(true) {
        return None;
    }
    Some(Card {
        id: v.get("id")?.as_str()?.to_string(),
        color: v
            .get("color")
            .and_then(|c| c.as_str())
            .unwrap_or("yellow")
            .to_string(),
        x: numf(v, "x", 0.0),
        y: numf(v, "y", 0.0),
        w: numf(v, "w", CARD_W),
        h: numf(v, "h", CARD_H),
        frame_id: v
            .get("frameId")
            .and_then(|f| f.as_str())
            .map(|s| s.to_string()),
        owner: v
            .get("owner")
            .and_then(|o| o.as_str())
            .map(|s| s.to_string()),
    })
}

fn column_of(color: &str) -> usize {
    COL_ORDER
        .iter()
        .position(|c| *c == color)
        .unwrap_or(COL_ORDER.len())
}

pub fn conn_pairs(conns: &[Value]) -> Vec<(String, String)> {
    conns
        .iter()
        .filter_map(|c| {
            let f = c.get("from")?.as_str()?.to_string();
            let t = c.get("to")?.as_str()?.to_string();
            Some((f, t))
        })
        .collect()
}

type Pos = HashMap<String, (f64, f64)>;

fn build_graph(
    cards: &[Card],
    conns: &[(String, String)],
) -> (
    Vec<String>,
    HashMap<String, Vec<String>>,
    HashMap<String, usize>,
) {
    let ids: Vec<String> = cards.iter().map(|c| c.id.clone()).collect();
    let idset: HashSet<&String> = ids.iter().collect();
    let mut children: HashMap<String, Vec<String>> =
        ids.iter().map(|id| (id.clone(), vec![])).collect();
    let mut indeg: HashMap<String, usize> = ids.iter().map(|id| (id.clone(), 0)).collect();
    for (f, t) in conns {
        if idset.contains(f) && idset.contains(t) && f != t {
            children.get_mut(f).unwrap().push(t.clone());
            *indeg.get_mut(t).unwrap() += 1;
        }
    }
    (ids, children, indeg)
}

fn col_positions(cards: &[Card], ox: f64, oy: f64, sp: f64) -> Pos {
    let mut pos = Pos::new();
    let mut sorted: Vec<&Card> = cards.iter().collect();
    sorted.sort_by(|a, b| {
        column_of(&a.color)
            .cmp(&column_of(&b.color))
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    // 欄寬 = 該欄最大卡寬(使用者拉大的卡不再蓋到隔壁欄);欄內 y 累計實際卡高
    let mut col_w: HashMap<usize, f64> = HashMap::new();
    for c in &sorted {
        let e = col_w.entry(column_of(&c.color)).or_insert(CARD_W);
        *e = e.max(c.w);
    }
    let mut cols: Vec<usize> = col_w.keys().copied().collect();
    cols.sort();
    let mut col_x: HashMap<usize, f64> = HashMap::new();
    let mut x = ox;
    for col in cols {
        col_x.insert(col, x);
        x += col_w[&col] + COL_GAP * sp;
    }
    let mut col_y: HashMap<usize, f64> = HashMap::new();
    for c in sorted {
        let col = column_of(&c.color);
        let y = *col_y.get(&col).unwrap_or(&oy);
        pos.insert(c.id.clone(), (col_x[&col], y));
        col_y.insert(col, y + c.h + ROW_GAP * sp);
    }
    pos
}

fn levels_longest(
    ids: &[String],
    children: &HashMap<String, Vec<String>>,
    indeg: &HashMap<String, usize>,
) -> HashMap<String, i64> {
    let mut roots: Vec<String> = ids
        .iter()
        .filter(|id| *indeg.get(*id).unwrap_or(&0) == 0)
        .cloned()
        .collect();
    if roots.is_empty() && !ids.is_empty() {
        roots.push(ids[0].clone());
    }
    let mut level: HashMap<String, i64> = HashMap::new();
    let mut q: std::collections::VecDeque<(String, i64)> =
        roots.into_iter().map(|r| (r, 0)).collect();
    let mut guard = 0;
    while let Some((id, lv)) = q.pop_front() {
        guard += 1;
        if guard > 20000 {
            break;
        }
        if *level.get(&id).unwrap_or(&-1) >= lv {
            continue;
        }
        level.insert(id.clone(), lv);
        for ch in children.get(&id).cloned().unwrap_or_default() {
            q.push_back((ch, lv + 1));
        }
    }
    for id in ids {
        level.entry(id.clone()).or_insert(0);
    }
    level
}

/// tidy-tree:父節點置中於子樹之上、同一父的子節點相鄰。
/// 杜絕舊版「逐層平鋪」造成的長距離跨欄連線(線壓過無關卡片)。
fn tree_positions(
    cards: &[Card],
    conns: &[(String, String)],
    ox: f64,
    oy: f64,
    dir: &str,
    sp: f64,
) -> Pos {
    let mut pos = Pos::new();
    if cards.is_empty() {
        return pos;
    }
    let by_id: HashMap<&String, &Card> = cards.iter().map(|c| (&c.id, c)).collect();
    let (ids, children, indeg) = build_graph(cards, conns);
    let level = levels_longest(&ids, &children, &indeg);
    // spanning tree:只保留 level 差 1 的邊;每個子節點只認第一個父(DAG 的其餘邊只畫線不佔位)
    let mut kids: HashMap<String, Vec<String>> = ids.iter().map(|i| (i.clone(), vec![])).collect();
    let mut has_parent: HashSet<String> = HashSet::new();
    let mut order = ids.clone();
    order.sort_by(|a, b| {
        let la = *level.get(a).unwrap_or(&0);
        let lb = *level.get(b).unwrap_or(&0);
        la.cmp(&lb).then_with(|| {
            let (ca, cb) = (by_id[a], by_id[b]);
            if dir == "LR" {
                ca.y.partial_cmp(&cb.y).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                ca.x.partial_cmp(&cb.x).unwrap_or(std::cmp::Ordering::Equal)
            }
        })
    });
    for p in &order {
        for ch in children.get(p).cloned().unwrap_or_default() {
            if *level.get(&ch).unwrap_or(&0) == *level.get(p).unwrap_or(&0) + 1
                && !has_parent.contains(&ch)
            {
                kids.get_mut(p).unwrap().push(ch.clone());
                has_parent.insert(ch);
            }
        }
    }
    // 遞迴:葉節點拿連續 slot,父節點置中於子節點 slot 範圍
    fn place(
        id: &str,
        kids: &HashMap<String, Vec<String>>,
        slot: &mut HashMap<String, f64>,
        next: &mut f64,
    ) -> f64 {
        let ch = kids.get(id).cloned().unwrap_or_default();
        let s = if ch.is_empty() {
            let s = *next;
            *next += 1.0;
            s
        } else {
            let centers: Vec<f64> = ch.iter().map(|c| place(c, kids, slot, next)).collect();
            (centers[0] + centers[centers.len() - 1]) / 2.0
        };
        slot.insert(id.to_string(), s);
        s
    }
    let mut slot: HashMap<String, f64> = HashMap::new();
    let mut next = 0.0_f64;
    for id in &order {
        if !has_parent.contains(id) && !slot.contains_key(id) {
            place(id, &kids, &mut slot, &mut next);
        }
    }
    // 座標:主軸 = level,交叉軸 = slot;間距取整批最大卡尺寸保證不疊
    let max_w = cards.iter().map(|c| c.w).fold(CARD_W, f64::max);
    let max_h = cards.iter().map(|c| c.h).fold(CARD_H, f64::max);
    let (slot_gap, level_gap) = if dir == "LR" {
        (max_h + 40.0 * sp, max_w + 50.0 * sp)
    } else {
        (max_w + 50.0 * sp, max_h + 40.0 * sp)
    };
    for id in &ids {
        let lv = *level.get(id).unwrap_or(&0) as f64;
        let s = *slot.get(id).unwrap_or(&0.0);
        let p = if dir == "LR" {
            (ox + lv * level_gap, oy + s * slot_gap)
        } else {
            (ox + s * slot_gap, oy + lv * level_gap)
        };
        pos.insert(id.clone(), p);
    }
    pos
}

fn radial_positions(cards: &[Card], conns: &[(String, String)], ox: f64, oy: f64, sp: f64) -> Pos {
    let mut pos = Pos::new();
    if cards.is_empty() {
        return pos;
    }
    let (ids, children, indeg) = build_graph(cards, conns);
    let center = ids
        .iter()
        .find(|id| *indeg.get(*id).unwrap_or(&0) == 0)
        .cloned()
        .unwrap_or_else(|| ids[0].clone());
    // BFS depth from center
    let mut level: HashMap<String, i64> = HashMap::new();
    level.insert(center.clone(), 0);
    let mut q: std::collections::VecDeque<(String, i64)> = std::collections::VecDeque::new();
    q.push_back((center.clone(), 0));
    let mut guard = 0;
    while let Some((id, lv)) = q.pop_front() {
        guard += 1;
        if guard > 20000 {
            break;
        }
        for ch in children.get(&id).cloned().unwrap_or_default() {
            if !level.contains_key(&ch) {
                level.insert(ch.clone(), lv + 1);
                q.push_back((ch, lv + 1));
            }
        }
    }
    for id in &ids {
        level.entry(id.clone()).or_insert(1);
    }
    // leaf counts for angular allocation
    let mut leaves: HashMap<String, f64> = HashMap::new();
    fn count_leaves(
        id: &str,
        children: &HashMap<String, Vec<String>>,
        level: &HashMap<String, i64>,
        leaves: &mut HashMap<String, f64>,
        seen: &mut HashSet<String>,
    ) -> f64 {
        if seen.contains(id) {
            return 1.0;
        }
        seen.insert(id.to_string());
        let ch: Vec<String> = children
            .get(id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|k| level.get(k).unwrap_or(&0) > level.get(id).unwrap_or(&0))
            .collect();
        let c = if ch.is_empty() {
            1.0
        } else {
            ch.iter()
                .map(|k| count_leaves(k, children, level, leaves, seen))
                .sum()
        };
        leaves.insert(id.to_string(), c);
        c
    }
    count_leaves(&center, &children, &level, &mut leaves, &mut HashSet::new());
    let mut ang: HashMap<String, f64> = HashMap::new();
    fn assign(
        id: &str,
        a0: f64,
        a1: f64,
        children: &HashMap<String, Vec<String>>,
        level: &HashMap<String, i64>,
        leaves: &HashMap<String, f64>,
        ang: &mut HashMap<String, f64>,
        seen: &mut HashSet<String>,
    ) {
        if seen.contains(id) {
            return;
        }
        seen.insert(id.to_string());
        ang.insert(id.to_string(), (a0 + a1) / 2.0);
        let ch: Vec<String> = children
            .get(id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|k| {
                level.get(k).unwrap_or(&0) > level.get(id).unwrap_or(&0) && !seen.contains(k)
            })
            .collect();
        let total: f64 = ch
            .iter()
            .map(|k| leaves.get(k).copied().unwrap_or(1.0))
            .sum::<f64>()
            .max(1.0);
        let mut a = a0;
        for k in ch {
            let span = (a1 - a0) * (leaves.get(&k).copied().unwrap_or(1.0) / total);
            assign(&k, a, a + span, children, level, leaves, ang, seen);
            a += span;
        }
    }
    assign(
        &center,
        -std::f64::consts::FRAC_PI_2,
        3.0 * std::f64::consts::FRAC_PI_2,
        &children,
        &level,
        &leaves,
        &mut ang,
        &mut HashSet::new(),
    );
    // 每層半徑:等差環距與「該層卡數所需周長」取大者 → 卡多的層自動撐大,不再擠成一圈疊住
    let ring = 200.0 + 40.0 * sp;
    let max_lv = level.values().copied().max().unwrap_or(0).max(0);
    let diag = (CARD_W * CARD_W + CARD_H * CARD_H).sqrt() + 30.0 * sp;
    let mut count_at: HashMap<i64, usize> = HashMap::new();
    for id in &ids {
        *count_at.entry(*level.get(id).unwrap_or(&1)).or_insert(0) += 1;
    }
    let mut radius_at: HashMap<i64, f64> = HashMap::new();
    let mut prev = 0.0_f64;
    for lv in 1..=max_lv {
        let n = *count_at.get(&lv).unwrap_or(&0) as f64;
        let needed = n * diag / (2.0 * std::f64::consts::PI);
        let r = (prev + ring).max(needed);
        radius_at.insert(lv, r);
        prev = r;
    }
    let span = prev.max(ring);
    let cx = ox + span;
    let cy = oy + span;
    for id in &ids {
        let lv = *level.get(id).unwrap_or(&0);
        if lv == 0 {
            pos.insert(id.clone(), (cx, cy));
        } else {
            let a = *ang.get(id).unwrap_or(&0.0);
            let r = *radius_at.get(&lv).unwrap_or(&ring);
            pos.insert(id.clone(), (cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    pos
}

/// SWOT 四象限:每象限排成 2 欄網格(不再是一直欄),象限大小取四象限最大,永不互疊。
fn quadrant_positions(cards: &[Card], ox: f64, oy: f64, sp: f64) -> Pos {
    let mut pos = Pos::new();
    let mut g: HashMap<&str, Vec<&Card>> = HashMap::new();
    for c in cards {
        let key = match c.color.as_str() {
            "green" | "yellow" | "blue" | "red" => c.color.as_str(),
            _ => "green",
        };
        g.entry(key).or_default().push(c);
    }
    let gy = 24.0 * sp;
    let gx = 24.0 * sp;
    const COLS: usize = 2;
    let empty = vec![];
    let quad_rows = |n: usize| n.div_ceil(COLS);
    let max_rows = ["green", "yellow", "blue", "red"]
        .iter()
        .map(|k| quad_rows(g.get(*k).unwrap_or(&empty).len()))
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let quad_w = COLS as f64 * (CARD_W + gx);
    let quad_h = max_rows * (CARD_H + gy);
    let place = |arr: &Vec<&Card>, x0: f64, y0: f64, pos: &mut Pos| {
        for (i, c) in arr.iter().enumerate() {
            let (col, row) = (i % COLS, i / COLS);
            pos.insert(
                c.id.clone(),
                (
                    x0 + col as f64 * (CARD_W + gx),
                    y0 + row as f64 * (CARD_H + gy),
                ),
            );
        }
    };
    let right_x = ox + quad_w + 80.0 * sp;
    let bot_y = oy + quad_h + 80.0;
    place(g.get("green").unwrap_or(&empty), ox, oy, &mut pos);
    place(g.get("yellow").unwrap_or(&empty), right_x, oy, &mut pos);
    place(g.get("blue").unwrap_or(&empty), ox, bot_y, &mut pos);
    place(g.get("red").unwrap_or(&empty), right_x, bot_y, &mut pos);
    pos
}

/// 經典魚骨圖:問題卡(魚頭)置於最右,想像的水平脊線往左延伸;
/// 主因卡沿脊線上、下交錯排列,各距脊線一段距離;每個主因的次因卡排在
/// 主因「外側」(上方骨往上、下方骨往下),並沿斜骨方向逐張往魚尾(左)
/// 錯開,形成教科書魚骨的斜骨視覺。每「對」骨(上、下各一)佔一條獨立
/// 水平帶,帶與帶不重疊、上下側以脊線分隔,輸出本身即保證無重疊。
fn fishbone_positions(
    cards: &[Card],
    conns: &[(String, String)],
    ox: f64,
    oy: f64,
    sp: f64,
) -> Pos {
    let mut pos = Pos::new();
    if cards.is_empty() {
        return pos;
    }
    let by_id: HashMap<&String, &Card> = cards.iter().map(|c| (&c.id, c)).collect();
    let (ids, children, _indeg) = build_graph(cards, conns);
    // connectors 方向 = 原因→結果:沒有任何流出邊的節點就是問題卡(魚頭)
    let head = ids
        .iter()
        .find(|id| children.get(*id).map(|c| c.is_empty()).unwrap_or(true))
        .cloned()
        .unwrap_or_else(|| ids[0].clone());
    let mut parents: HashMap<String, Vec<String>> =
        ids.iter().map(|id| (id.clone(), vec![])).collect();
    for f in &ids {
        for t in children.get(f).cloned().unwrap_or_default() {
            parents.get_mut(&t).unwrap().push(f.clone());
        }
    }
    // 從魚頭沿 結果→原因 反向 BFS 推層級:0=問題、1=主因、2+=次因
    let mut level: HashMap<String, i64> = HashMap::new();
    level.insert(head.clone(), 0);
    let mut q: std::collections::VecDeque<(String, i64)> = std::collections::VecDeque::new();
    q.push_back((head.clone(), 0));
    let mut guard = 0;
    while let Some((id, lv)) = q.pop_front() {
        guard += 1;
        if guard > 20000 {
            break;
        }
        for p in parents.get(&id).cloned().unwrap_or_default() {
            if !level.contains_key(&p) {
                level.insert(p.clone(), lv + 1);
                q.push_back((p, lv + 1));
            }
        }
    }
    for id in &ids {
        level.entry(id.clone()).or_insert(1);
    }
    // 主因(骨):依卡片原始順序;偶數索引排脊線上方、奇數排下方 → 上下交錯
    let bones: Vec<String> = ids
        .iter()
        .filter(|id| **id != head && *level.get(*id).unwrap_or(&1) == 1)
        .cloned()
        .collect();
    if bones.is_empty() {
        pos.insert(head.clone(), (ox, oy));
        return pos;
    }
    // 次因歸骨:沿 原因→結果 邊往魚頭方向走一步(level 恰好少 1 的 child)。
    // 淺層先處理,深層才查得到所屬骨;DAG 雜邊找不到時退而掛在已知骨或第 0 根骨。
    let mut bone_of: HashMap<String, usize> = bones
        .iter()
        .enumerate()
        .map(|(i, b)| (b.clone(), i))
        .collect();
    let mut sub_ids: Vec<String> = ids
        .iter()
        .filter(|id| *level.get(*id).unwrap_or(&1) >= 2)
        .cloned()
        .collect();
    sub_ids.sort_by_key(|id| *level.get(id).unwrap_or(&1));
    let mut bone_subs: Vec<Vec<String>> = vec![vec![]; bones.len()];
    for id in sub_ids {
        let lv = *level.get(&id).unwrap_or(&1);
        let ch = children.get(&id).cloned().unwrap_or_default();
        let b = ch
            .iter()
            .find_map(|c| {
                if *level.get(c).unwrap_or(&1) + 1 == lv {
                    bone_of.get(c).copied()
                } else {
                    None
                }
            })
            .or_else(|| ch.iter().find_map(|c| bone_of.get(c).copied()))
            .unwrap_or(0);
        bone_of.insert(id.clone(), b);
        bone_subs[b].push(id);
    }
    // 幾何參數:沿骨往外每張的縱距、斜骨往魚尾的橫向錯位、主因距脊線距離、骨帶間距
    let max_w = cards.iter().map(|c| c.w).fold(CARD_W, f64::max);
    let max_h = cards.iter().map(|c| c.h).fold(CARD_H, f64::max);
    let pitch = max_h + 40.0 * sp;
    let dx = 60.0 * sp;
    let main_off = 70.0 * sp;
    let band_gap = 60.0 * sp;
    let head_card = by_id[&head];
    // 脊線高度:預留上方骨最深的外側延伸,確保最上面的次因仍不超出 oy
    let mut above_ext = head_card.h / 2.0;
    for (i, subs) in bone_subs.iter().enumerate() {
        if i % 2 == 0 {
            above_ext = above_ext.max(main_off + subs.len() as f64 * pitch + max_h);
        }
    }
    let spine_y = oy + above_ext;
    // 由左(魚尾)往右逐對放置;帶寬涵蓋該對最深的斜向錯位,主因卡靠帶右緣
    let npairs = bones.len().div_ceil(2);
    let mut cursor = ox;
    for p in 0..npairs {
        let lean = dx
            * (p * 2..(p * 2 + 2).min(bones.len()))
                .map(|i| bone_subs[i].len())
                .max()
                .unwrap_or(0) as f64;
        let main_x = cursor + lean;
        for i in p * 2..(p * 2 + 2).min(bones.len()) {
            let above = i % 2 == 0;
            let mc = by_id[&bones[i]];
            let my = if above {
                spine_y - main_off - mc.h
            } else {
                spine_y + main_off
            };
            pos.insert(bones[i].clone(), (main_x, my));
            for (j, sid) in bone_subs[i].iter().enumerate() {
                let sc = by_id[sid];
                let out = (j as f64 + 1.0) * pitch;
                let sy = if above {
                    spine_y - main_off - out - sc.h
                } else {
                    spine_y + main_off + out
                };
                pos.insert(sid.clone(), (main_x - (j as f64 + 1.0) * dx, sy));
            }
        }
        cursor = main_x + max_w + band_gap;
    }
    // 魚頭:最右、垂直置中於脊線
    pos.insert(head.clone(), (cursor, spine_y - head_card.h / 2.0));
    pos
}

fn gantt_positions(cards: &[Card], conns: &[(String, String)], ox: f64, oy: f64, sp: f64) -> Pos {
    let mut pos = Pos::new();
    if cards.is_empty() {
        return pos;
    }
    let by_id: HashMap<&String, &Card> = cards.iter().map(|c| (&c.id, c)).collect();
    let (ids, children, indeg) = build_graph(cards, conns);
    let mut indeg_c: HashMap<String, i64> =
        indeg.iter().map(|(k, v)| (k.clone(), *v as i64)).collect();
    let mut queue: Vec<String> = ids
        .iter()
        .filter(|id| *indeg_c.get(*id).unwrap_or(&0) == 0)
        .cloned()
        .collect();
    queue.sort_by(|a, b| {
        by_id[a]
            .x
            .partial_cmp(&by_id[b].x)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut order: Vec<String> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let mut guard = 0;
    while let Some(id) = queue.first().cloned() {
        queue.remove(0);
        guard += 1;
        if guard > 20000 {
            break;
        }
        if seen.contains(&id) {
            continue;
        }
        seen.insert(id.clone());
        order.push(id.clone());
        for t in children.get(&id).cloned().unwrap_or_default() {
            let e = indeg_c.entry(t.clone()).or_insert(0);
            *e -= 1;
            if *e <= 0 {
                queue.push(t);
            }
        }
    }
    for id in &ids {
        if !seen.contains(id) {
            order.push(id.clone());
        }
    }
    let mut row_of: HashMap<String, usize> = HashMap::new();
    for id in &order {
        let o = by_id[id]
            .owner
            .clone()
            .unwrap_or_else(|| "未指派".to_string());
        let next = row_of.len();
        row_of.entry(o).or_insert(next);
    }
    let gx = CARD_W + 40.0 * sp;
    let gy = CARD_H + 30.0 * sp;
    for (col, id) in order.iter().enumerate() {
        let o = by_id[id]
            .owner
            .clone()
            .unwrap_or_else(|| "未指派".to_string());
        pos.insert(
            id.clone(),
            (
                ox + col as f64 * gx,
                oy + *row_of.get(&o).unwrap_or(&0) as f64 * gy,
            ),
        );
    }
    pos
}

/// 最終防線:layout 演算法之外的任何殘餘重疊(radial 角度過近、奇形怪狀的圖)
/// 由上而下掃,疊到的卡往下推。最多 6 輪,每輪 O(n^2)(n = 單一 frame 卡數,夠小)。
fn resolve_collisions(cards: &[Card], pos: &mut Pos, sp: f64) {
    let pad = 16.0 * sp;
    for _ in 0..6 {
        let mut moved = false;
        let mut order: Vec<&Card> = cards.iter().filter(|c| pos.contains_key(&c.id)).collect();
        order.sort_by(|a, b| {
            let (_, ay) = pos[&a.id];
            let (_, by) = pos[&b.id];
            ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in 0..order.len() {
            for j in i + 1..order.len() {
                let (ax, ay) = pos[&order[i].id];
                let (bx, by) = pos[&order[j].id];
                let (aw, ah) = (order[i].w, order[i].h);
                let (bw, bh) = (order[j].w, order[j].h);
                if ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah {
                    pos.insert(order[j].id.clone(), (bx, ay + ah + pad));
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

fn layout_positions(
    type_key: &str,
    cards: &[Card],
    conns: &[(String, String)],
    ox: f64,
    oy: f64,
    sp: f64,
) -> Pos {
    let bt = board_type(type_key);
    let mut pos = match bt.layout {
        "tree" => tree_positions(cards, conns, ox, oy, bt.dir, sp),
        "radial" => radial_positions(cards, conns, ox, oy, sp),
        "quadrant" => quadrant_positions(cards, ox, oy, sp),
        "fishbone" => fishbone_positions(cards, conns, ox, oy, sp),
        "gantt" => gantt_positions(cards, conns, ox, oy, sp),
        _ => col_positions(cards, ox, oy, sp),
    };
    resolve_collisions(cards, &mut pos, sp);
    pos
}

pub struct Frame {
    pub id: String,
    pub typ: String,
    pub x: f64,
    pub y: f64,
}
pub fn frame_from(v: &Value) -> Option<Frame> {
    Some(Frame {
        id: v.get("id")?.as_str()?.to_string(),
        typ: v
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("meeting")
            .to_string(),
        x: numf(v, "x", 80.0),
        y: numf(v, "y", 80.0),
    })
}

pub struct FramePlace {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Returns (card positions, frame placements). Frameless boards => one whole-board layout.
/// frame 先在原點排版量尺寸,再整批 re-pack(列式、超寬換行)—— 舊版只放大不移位,
/// frame 長大後會蓋到建立時排在右邊的鄰居;現在 frame 永不互疊。
pub fn tidy(
    meta_type: &str,
    shapes: &[Value],
    conns_v: &[Value],
    frames_v: &[Value],
    sp: f64,
) -> (Vec<(String, f64, f64)>, Vec<FramePlace>) {
    let cards: Vec<Card> = shapes.iter().filter_map(card_from).collect();
    let conns = conn_pairs(conns_v);
    let frames: Vec<Frame> = frames_v.iter().filter_map(frame_from).collect();
    let mut out_pos: Vec<(String, f64, f64)> = vec![];
    let mut out_frames: Vec<FramePlace> = vec![];
    if frames.is_empty() {
        let pos = layout_positions(meta_type, &cards, &conns, X0, Y0, sp);
        for (id, (x, y)) in pos {
            out_pos.push((id, x, y));
        }
        return (out_pos, out_frames);
    }
    // pass 1:每個 frame 在 (0,0) 排版,量出內容尺寸
    struct Laid {
        frame: Frame,
        rel: Pos,
        cards: Vec<Card>,
        w: f64,
        h: f64,
    }
    let mut laid: Vec<Laid> = vec![];
    for f in &frames {
        let fcards: Vec<Card> = cards
            .iter()
            .filter(|c| c.frame_id.as_deref() == Some(f.id.as_str()))
            .cloned()
            .collect();
        let rel = layout_positions(&f.typ, &fcards, &conns, 0.0, 0.0, sp);
        let mut max_x = 0.0_f64;
        let mut max_y = 0.0_f64;
        for c in &fcards {
            if let Some((x, y)) = rel.get(&c.id) {
                max_x = max_x.max(x + c.w);
                max_y = max_y.max(y + c.h);
            }
        }
        let w = (max_x + FRAME_PAD * 2.0).max(440.0);
        let h = (max_y + FRAME_PAD + FRAME_HEAD).max(300.0);
        laid.push(Laid {
            frame: Frame {
                id: f.id.clone(),
                typ: f.typ.clone(),
                x: f.x,
                y: f.y,
            },
            rel,
            cards: fcards,
            w,
            h,
        });
    }
    // pass 2:依目前閱讀順序(y 再 x)re-pack 成列,超寬換行 → frame 永不互疊
    laid.sort_by(|a, b| {
        a.frame
            .y
            .partial_cmp(&b.frame.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.frame
                    .x
                    .partial_cmp(&b.frame.x)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    const FRAME_GAP: f64 = 90.0;
    const MAX_ROW_W: f64 = 3200.0;
    let (mut cx, mut cy) = (80.0_f64, 80.0_f64);
    let mut row_h = 0.0_f64;
    for l in &laid {
        if cx > 80.0 && cx + l.w > MAX_ROW_W {
            cx = 80.0;
            cy += row_h + FRAME_GAP;
            row_h = 0.0;
        }
        for c in &l.cards {
            if let Some((rx, ry)) = l.rel.get(&c.id) {
                out_pos.push((c.id.clone(), cx + FRAME_PAD + rx, cy + FRAME_HEAD + ry));
            }
        }
        out_frames.push(FramePlace {
            id: l.frame.id.clone(),
            x: cx,
            y: cy,
            w: l.w,
            h: l.h,
        });
        cx += l.w + FRAME_GAP;
        row_h = row_h.max(l.h);
    }
    (out_pos, out_frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk(id: &str, color: &str, w: f64, h: f64) -> Card {
        Card {
            id: id.into(),
            color: color.into(),
            x: 0.0,
            y: 0.0,
            w,
            h,
            frame_id: None,
            owner: None,
        }
    }

    fn rects_disjoint(cards: &[Card], pos: &Pos) -> bool {
        let r: Vec<(f64, f64, f64, f64)> = cards
            .iter()
            .filter_map(|c| pos.get(&c.id).map(|(x, y)| (*x, *y, c.w, c.h)))
            .collect();
        for i in 0..r.len() {
            for j in i + 1..r.len() {
                let (ax, ay, aw, ah) = r[i];
                let (bx, by, bw, bh) = r[j];
                if ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn columns_respect_actual_card_sizes() {
        let cards = vec![
            mk("a", "yellow", 200.0, 200.0),
            mk("b", "yellow", 200.0, 420.0), // 使用者拉大的卡
            mk("c", "yellow", 200.0, 200.0),
            mk("d", "green", 360.0, 200.0),
            mk("e", "green", 200.0, 200.0),
        ];
        let pos = col_positions(&cards, 0.0, 0.0, 1.0);
        assert!(rects_disjoint(&cards, &pos));
    }

    #[test]
    fn tree_parents_centered_over_children_and_disjoint() {
        let cards: Vec<Card> = ["root", "a", "b", "c", "a1", "a2", "b1"]
            .iter()
            .map(|id| mk(id, "yellow", 200.0, 200.0))
            .collect();
        let conns = vec![
            ("root".to_string(), "a".to_string()),
            ("root".to_string(), "b".to_string()),
            ("root".to_string(), "c".to_string()),
            ("a".to_string(), "a1".to_string()),
            ("a".to_string(), "a2".to_string()),
            ("b".to_string(), "b1".to_string()),
        ];
        let pos = tree_positions(&cards, &conns, 0.0, 0.0, "TB", 1.0);
        assert!(rects_disjoint(&cards, &pos));
        // a 必須水平置中在 a1、a2 之間;a1 a2 同一列
        let (ax, _) = pos["a"];
        let (a1x, a1y) = pos["a1"];
        let (a2x, a2y) = pos["a2"];
        assert!((ax - (a1x + a2x) / 2.0).abs() < 1.0, "parent centered");
        assert_eq!(a1y, a2y);
    }

    #[test]
    fn radial_dense_level_grows_radius() {
        let mut cards = vec![mk("hub", "blue", 200.0, 200.0)];
        let mut conns = vec![];
        for i in 0..14 {
            let id = format!("n{}", i);
            cards.push(mk(&id, "green", 200.0, 200.0));
            conns.push(("hub".to_string(), id));
        }
        let pos = radial_positions(&cards, &conns, 0.0, 0.0, 1.0);
        assert!(rects_disjoint(&cards, &pos));
    }

    #[test]
    fn quadrant_grid_two_cols_disjoint() {
        let mut cards = vec![];
        for i in 0..7 {
            cards.push(mk(&format!("g{}", i), "green", 200.0, 200.0));
        }
        for i in 0..3 {
            cards.push(mk(&format!("r{}", i), "red", 200.0, 200.0));
        }
        let pos = quadrant_positions(&cards, 0.0, 0.0, 1.0);
        assert!(rects_disjoint(&cards, &pos));
        // green 7 張要排成 2 欄,不是 7 張一直欄
        let xs: HashSet<i64> = (0..7)
            .map(|i| pos[&format!("g{}", i)].0 as i64)
            .collect();
        assert_eq!(xs.len(), 2);
    }

    #[test]
    fn fishbone_classic_shape() {
        // 1 問題(blue) + 4 主因(green) + 各 2 次因(yellow/red);connectors 方向 = 原因→結果
        let mut cards = vec![mk("prob", "blue", 200.0, 200.0)];
        let mut conns: Vec<(String, String)> = vec![];
        for m in 0..4 {
            let mid = format!("m{}", m);
            cards.push(mk(&mid, "green", 200.0, 200.0));
            conns.push((mid.clone(), "prob".to_string()));
            for s in 0..2 {
                let sid = format!("m{}s{}", m, s);
                cards.push(mk(&sid, if s == 0 { "yellow" } else { "red" }, 200.0, 200.0));
                conns.push((sid.clone(), mid.clone()));
            }
        }
        let pos = fishbone_positions(&cards, &conns, 0.0, 0.0, 1.0);
        assert_eq!(pos.len(), cards.len(), "all cards placed");
        // 座標表(cargo test fishbone_classic_shape -- --nocapture 人工檢查形狀)
        let mut rows: Vec<&Card> = cards.iter().collect();
        rows.sort_by(|a, b| {
            pos[&a.id]
                .0
                .partial_cmp(&pos[&b.id].0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for c in &rows {
            let (x, y) = pos[&c.id];
            println!("{:<6} {:<7} x={:>6.0} y={:>6.0}", c.id, c.color, x, y);
        }
        // (a) 無重疊
        assert!(rects_disjoint(&cards, &pos), "cards overlap");
        // (b) 問題卡 x 最大 → 魚頭在最右
        let (px, py) = pos["prob"];
        for c in &cards {
            if c.id != "prob" {
                assert!(pos[&c.id].0 < px, "{} 應在問題卡左側", c.id);
            }
        }
        // (c) 主因卡沿脊線(問題卡垂直中心)上下交錯:上、下、上、下
        let spine = py + 100.0;
        for m in 0..4 {
            let (_, my) = pos[&format!("m{}", m)];
            if m % 2 == 0 {
                assert!(my + 200.0 < spine, "m{} 應整張在脊線上方", m);
            } else {
                assert!(my > spine, "m{} 應整張在脊線下方", m);
            }
        }
        // (d) 次因與主因同側且更外側(上方骨更上、下方骨更下),並沿斜骨往魚尾錯開
        for m in 0..4 {
            let (mx, my) = pos[&format!("m{}", m)];
            let (mut prev_x, mut prev_y) = (mx, my);
            for s in 0..2 {
                let (sx, sy) = pos[&format!("m{}s{}", m, s)];
                if m % 2 == 0 {
                    assert!(sy + 200.0 <= my, "m{}s{} 應在主因上方外側", m, s);
                    assert!(sy < prev_y, "m{}s{} 應比前一張更外側", m, s);
                } else {
                    assert!(sy >= my + 200.0, "m{}s{} 應在主因下方外側", m, s);
                    assert!(sy > prev_y, "m{}s{} 應比前一張更外側", m, s);
                }
                assert!(sx < prev_x, "m{}s{} 應沿斜骨往魚尾(左)錯開", m, s);
                (prev_x, prev_y) = (sx, sy);
            }
        }
    }

    #[test]
    fn safety_net_separates_any_residual_overlap() {
        let cards = vec![
            mk("a", "yellow", 200.0, 200.0),
            mk("b", "yellow", 200.0, 200.0),
        ];
        let mut pos = Pos::new();
        pos.insert("a".into(), (0.0, 0.0));
        pos.insert("b".into(), (50.0, 30.0)); // 故意疊
        resolve_collisions(&cards, &mut pos, 1.0);
        assert!(rects_disjoint(&cards, &pos));
    }

    #[test]
    fn tidy_repacks_frames_no_overlap() {
        // 兩個 frame 起始互疊,各 4 張卡
        let frames = vec![
            json!({"id":"f1","type":"meeting","x":80.0,"y":80.0,"w":480.0,"h":320.0}),
            json!({"id":"f2","type":"flow","x":200.0,"y":120.0,"w":480.0,"h":320.0}),
        ];
        let mut shapes = vec![];
        for i in 0..4 {
            shapes.push(json!({"id":format!("c{}",i),"type":"sticky","color":"yellow","x":0.0,"y":0.0,"w":200.0,"h":200.0,"frameId":"f1"}));
            shapes.push(json!({"id":format!("d{}",i),"type":"sticky","color":"green","x":0.0,"y":0.0,"w":200.0,"h":200.0,"frameId":"f2"}));
        }
        let (pos, fr) = tidy("meeting", &shapes, &[], &frames, 1.0);
        assert_eq!(fr.len(), 2);
        // frame 矩形必須不相交
        let r: Vec<(f64, f64, f64, f64)> = fr.iter().map(|f| (f.x, f.y, f.w, f.h)).collect();
        assert!(
            !(r[0].0 < r[1].0 + r[1].2
                && r[1].0 < r[0].0 + r[0].2
                && r[0].1 < r[1].1 + r[1].3
                && r[1].1 < r[0].1 + r[0].3),
            "frames overlap: {:?}",
            r
        );
        // 每張卡要落在自己 frame 的矩形內(c* -> f1, d* -> f2)
        let fbyid: HashMap<&str, &FramePlace> = fr.iter().map(|f| (f.id.as_str(), f)).collect();
        for (id, x, y) in &pos {
            let f = if id.starts_with('c') { fbyid["f1"] } else { fbyid["f2"] };
            assert!(
                *x >= f.x && *y >= f.y && *x + 200.0 <= f.x + f.w + 1.0 && *y + 200.0 <= f.y + f.h + 1.0,
                "card {} ({},{}) outside frame {:?}",
                id, x, y, (f.x, f.y, f.w, f.h)
            );
        }
    }

    #[test]
    fn all_board_types_produce_disjoint_layouts() {
        for t in [
            "meeting", "orgchart", "flow", "architecture", "mindmap",
            "kanban", "swot", "timeline", "fishbone", "gantt",
        ] {
            let colors = ["yellow", "green", "blue", "red"];
            let cards: Vec<Card> = (0..13)
                .map(|i| {
                    let mut c = mk(&format!("n{}", i), colors[i % 4], 200.0, 200.0);
                    if i == 5 {
                        c.w = 380.0;
                        c.h = 300.0; // 一張使用者拉大的卡
                    }
                    c
                })
                .collect();
            let mut conns: Vec<(String, String)> = (1..13)
                .map(|i| (format!("n{}", (i - 1) / 2), format!("n{}", i)))
                .collect(); // 二元樹
            conns.push(("n3".into(), "n12".into())); // 非樹邊(DAG)
            let pos = layout_positions(t, &cards, &conns, 0.0, 0.0, 1.0);
            assert_eq!(pos.len(), cards.len(), "{}: all cards placed", t);
            assert!(rects_disjoint(&cards, &pos), "{}: cards overlap", t);
        }
    }
}
