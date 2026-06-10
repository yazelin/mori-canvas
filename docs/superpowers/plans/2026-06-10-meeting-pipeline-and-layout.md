# Meeting Pipeline Staging + Layout Overlap Elimination Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 即時會議逐字稿先經過獨立的「清稿」前處理階段（規則 + LLM，仿 mori-ear cleanup.rs）再進 board-agent，杜絕贅字冗詞/斷錯句寫進卡片；同時重寫排版引擎消除卡片、frame、連線互疊。

**Architecture:** 兩段式 AI pipeline — stage 1 `cleanup.rs`（規則清贅字 → LLM 最小幅度清稿，失敗 fallback 原文）在三個入口（/api/agent、/api/voice、/api/visualize）於 chunk 之前執行；stage 2 沿用 board-agent。排版引擎 `layout.rs` 改用卡片實際尺寸、tree 改 tidy-tree（父置中於子之上）、radial 加最小半徑、quadrant 改 mini-grid、`tidy()` 增加 frame re-packing（frame 會重新定位不再互蓋）、最後加 collision safety-net 與重疊回歸測試。

**Tech Stack:** Rust (warp + yrs + reqwest)、React + Konva client、Groq gpt-oss-120b → Ollama qwen3 cascade、Render Docker auto-deploy。

**Repo:** /home/ct/mori-universe/mori-canvas（public，Render 於 main push 後自動部署 https://mori-canvas.onrender.com/ ）

**Workflow:** trunk-based — 每個 Phase 一條短命 branch + PR + auto-merge，CI 綠才進 main。commit 訊息無 emoji。

---

## Phase 0 — PR CI gate

### Task 1: 新增 ci.yml（PR 必跑 cargo test + client build）

目前只有 release.yml（tag 觸發），PR 無 CI gate，auto-merge 沒東西可等。

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: 寫 workflow**

```yaml
name: ci
on:
  pull_request:
  push:
    branches: [main]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 20 }
      - run: npm ci
      - run: npm run build:client
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: server-rs }
      - run: cargo test --manifest-path server-rs/Cargo.toml
```

- [ ] **Step 2: 本地先確認兩個指令都過**

Run: `npm run build:client && cargo test --manifest-path server-rs/Cargo.toml`
Expected: client build 成功、所有既有測試 PASS

- [ ] **Step 3: Commit + 開 branch `ci/pr-gate` + PR + auto-merge**

```bash
git checkout -b ci/pr-gate origin/main
git add .github/workflows/ci.yml
git commit -m "ci: run cargo test + client build on every PR"
git push -u origin ci/pr-gate
gh pr create --fill && gh pr merge --auto --squash
```

---

## Phase 1 — 兩段式 AI：transcript 清稿前處理（branch `feat/transcript-cleanup`）

### Task 2: 規則層 scrub_fillers()（TDD）

**Files:**
- Create: `server-rs/src/cleanup.rs`
- Modify: `server-rs/src/lib.rs`（加 `pub mod cleanup;`，放在其他 `pub mod` 旁）

- [ ] **Step 1: 寫失敗測試（cleanup.rs 內 #[cfg(test)]）**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collapses_repeated_chars_and_words() {
        assert_eq!(scrub_fillers("對對對對,就這樣"), "對,就這樣");
        assert_eq!(scrub_fillers("那個那個我們先報價"), "那個我們先報價");
    }
    #[test]
    fn strips_leading_interjections_per_segment() {
        assert_eq!(scrub_fillers("嗯,我們下週交付"), "我們下週交付");
        assert_eq!(scrub_fillers("呃 這個案子。欸,先做 demo"), "這個案子。先做 demo");
    }
    #[test]
    fn keeps_meaningful_text_untouched() {
        let s = "客戶擔心櫃台人員不會用後台,我說會做教學影片。";
        assert_eq!(scrub_fillers(s), s);
    }
}
```

- [ ] **Step 2: 跑測試確認失敗**（`cargo test --manifest-path server-rs/Cargo.toml cleanup`，Expected: compile error / FAIL）

- [ ] **Step 3: 實作（保守規則,只動高確信贅字）**

```rust
//! Stage-1 前處理:把 STT 逐字稿清成乾淨文字,再交給 board-agent(stage 2)。
//! 規則層只動「高確信」贅字;語意層交給 LLM(cleanup_transcript)。
use crate::llm::{self, Msg};

/// 連續重複的單字 / 雙字詞收斂成一次;段首語助詞(嗯呃欸喔哦啊)移除。
pub fn scrub_fillers(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    // 1) 連續同字 3+ 次 → 1 次;同字 2 次保留(疊字詞如「謝謝」常見)
    let mut pass1: Vec<char> = Vec::with_capacity(chars.len());
    for &c in &chars {
        let n = pass1.len();
        if n >= 2 && pass1[n - 1] == c && pass1[n - 2] == c && is_cjk(c) {
            continue;
        }
        pass1.push(c);
    }
    // 例外:「對對」「好好」等口語重複兩次也收斂成一次(白名單)
    let doubles = ["對對", "好好好", "好好", "是是", "恩恩", "嗯嗯"];
    let mut s: String = pass1.into_iter().collect();
    for d in doubles {
        let single: String = d.chars().take(1).collect();
        while s.contains(d) {
            s = s.replace(d, &single);
        }
    }
    // 2) 連續重複的雙字詞(那個那個 / 就是就是)→ 一次
    let cs: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(cs.len());
    let mut i = 0;
    while i < cs.len() {
        if i + 4 <= cs.len()
            && cs[i] == cs[i + 2]
            && cs[i + 1] == cs[i + 3]
            && is_cjk(cs[i])
            && is_cjk(cs[i + 1])
        {
            out.push(cs[i]);
            out.push(cs[i + 1]);
            i += 4;
            // 吃掉更多重複
            while i + 2 <= cs.len() && cs[i] == out[out.len() - 2] && cs[i + 1] == out[out.len() - 1] {
                i += 2;
            }
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    let s: String = out.into_iter().collect();
    // 3) 每個句段開頭的語助詞 + 跟隨的逗號/空白移除
    const FILLER_HEAD: [char; 6] = ['嗯', '呃', '欸', '喔', '哦', '啊'];
    s.split_inclusive(['。', '!', '?', '!', '?', ';', ';', '\n'])
        .map(|seg| {
            let mut t = seg.trim_start();
            loop {
                let mut it = t.chars();
                match it.next() {
                    Some(c) if FILLER_HEAD.contains(&c) => {
                        t = it.as_str().trim_start_matches([',', ',', ' ', '、']).trim_start();
                    }
                    _ => break,
                }
            }
            t.to_string()
        })
        .collect::<Vec<_>>()
        .join("")
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}
```

- [ ] **Step 4: 跑測試到綠**（測試的期望值以實作行為微調措辭,但三類行為必須成立:3+ 重複收斂、段首語助詞移除、正常句不動）

- [ ] **Step 5: Commit** `feat(cleanup): rule-based filler scrubbing for stt transcripts`

### Task 3: LLM 清稿層 cleanup_transcript() + prompt 檔

**Files:**
- Create: `prompts/transcript-cleanup.md`
- Modify: `server-rs/src/cleanup.rs`

- [ ] **Step 1: 寫 prompt（仿 mori-ear cleanup.rs,加贅字冗詞與會議情境）**

```markdown
<!-- stage-1 清稿:在 AI 畫卡之前,先把 STT 逐字稿整理成乾淨的會議文字。改完存檔下個請求生效。 -->
{{include:common}}

你是會議逐字稿清稿員。輸入是語音辨識(STT)的原始輸出,常有錯字、斷錯句、口語贅字。把它清成乾淨的會議記錄文字,規則:

1. 修錯字:同音字、相近詞(例:「預月」→「預約」)。
2. 重新斷句:依語意補標點(逗號、句號、問號),把黏在一起的句子切開、把被切碎的句子接回。
3. 刪贅字冗詞:嗯、呃、欸、那個、就是說、然後然後、對對對、這樣子、的部分、的動作…等口語填充詞;重複的詞只留一次。
4. 不增不減語意:不改寫、不縮寫、不擴寫、不加入原文沒有的內容;專有名詞、人名、數字、金額原樣保留。
5. 整段都是無意義的語助詞或雜音時輸出空字串。

只輸出清好的繁體中文純文字,不要解釋、不要前言、不要 markdown 圍欄。
```

- [ ] **Step 2: 實作 cleanup_transcript（長稿切塊清、失敗 fallback）**

```rust
/// 整段逐字稿太長時切成 ~1500 字塊逐塊清(LLM context / 速度),完成後接回。
fn cleanup_blocks(raw: &str) -> Vec<String> {
    const MAX: usize = 1500;
    let mut blocks = vec![];
    let mut cur = String::new();
    for unit in raw.split_inclusive(['\n', '。', '!', '?', '!', '?']) {
        if !cur.is_empty() && cur.chars().count() + unit.chars().count() > MAX {
            blocks.push(std::mem::take(&mut cur));
        }
        cur.push_str(unit);
    }
    if !cur.trim().is_empty() {
        blocks.push(cur);
    }
    blocks
}

/// Stage-1 清稿:規則層 → LLM 層。LLM 失敗就用規則層結果(永不擋住 stage 2)。
/// 回傳 (清好的文字, 是否套到 LLM 層)。輸入太短(< 10 字,多半是指令)直接跳過。
pub async fn cleanup_transcript(raw: &str, local_only: bool, opts: &llm::LlmOpts) -> (String, bool) {
    let scrubbed = scrub_fillers(raw);
    if scrubbed.chars().count() < 10 {
        return (scrubbed, false);
    }
    let sys = crate::prompts::prompt("transcript-cleanup");
    let mut cleaned = String::new();
    for block in cleanup_blocks(&scrubbed) {
        let msgs = [
            Msg { role: "system", content: sys.clone() },
            Msg { role: "user", content: block.clone() },
        ];
        match llm::chat(&msgs, false, local_only, opts).await {
            Ok((t, _)) => {
                let t = crate::agent::strip_think(&t);
                if !t.trim().is_empty() {
                    cleaned.push_str(t.trim());
                    cleaned.push('\n');
                }
            }
            Err(_) => return (scrubbed, false), // LLM 掛了 → 整段用規則層結果
        }
    }
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        (scrubbed, false)
    } else {
        (cleaned, true)
    }
}
```

（`strip_think` 若不在 agent.rs 公開,執行時找到實際位置並 pub 化;`prompts::prompt` 已存在。）

- [ ] **Step 3: 單元測試 cleanup_blocks 切塊邏輯**

```rust
#[test]
fn blocks_split_on_sentence_ends_around_1500_chars() {
    let raw = "句子。".repeat(900); // 2700 chars
    let blocks = cleanup_blocks(&raw);
    assert!(blocks.len() >= 2);
    assert!(blocks.iter().all(|b| b.chars().count() <= 1503));
    assert_eq!(blocks.concat(), raw);
}
```

- [ ] **Step 4: cargo test 綠 + Commit** `feat(cleanup): llm transcript cleanup stage with rule-layer fallback`

### Task 4: 三個入口接上 stage-1（/api/agent、/api/voice、/api/visualize）

**Files:**
- Modify: `server-rs/src/lib.rs`（三個 endpoint）

- [ ] **Step 1: /api/agent — transcript 取出後、run_agent_turn 前清稿**

在 lib.rs `agent_ep`（transcript 驗空之後、`run_agent_turn` 之前）插入；body 可帶 `"cleanup": false` 跳過：

```rust
let do_cleanup = body.get("cleanup").and_then(|v| v.as_bool()).unwrap_or(true);
let (transcript, cleaned) = if do_cleanup {
    cleanup::cleanup_transcript(&transcript, s.local_only, &llm).await
} else {
    (transcript, false)
};
if transcript.trim().is_empty() {
    return Ok(warp::reply::json(&json!({ "ok": true, "stickies": 0, "skipped": "整段都是語助詞" })));
}
```

回應 JSON 加 `res["cleaned"] = json!(cleaned)`。

- [ ] **Step 2: /api/voice — STT 之後清稿,回應帶 raw + cleaned**

```rust
let raw_transcript = transcript.clone();
let (transcript, cleaned) =
    cleanup::cleanup_transcript(&transcript, s.local_only, &llm).await;
if transcript.trim().is_empty() {
    return Ok(warp::reply::json(
        &json!({ "ok": true, "transcript": "", "rawTranscript": raw_transcript, "stickies": 0 }),
    ));
}
// ...run_agent_turn(&room, &transcript, ...)
res["transcript"] = json!(transcript);
res["rawTranscript"] = json!(raw_transcript);
res["cleaned"] = json!(cleaned);
```

- [ ] **Step 3: /api/visualize — chunk_transcript 之前整段清一次**

```rust
let (transcript, _cleaned) = if body.get("cleanup").and_then(|v| v.as_bool()).unwrap_or(true) {
    cleanup::cleanup_transcript(&transcript, s.local_only, &llm).await
} else {
    (transcript, false)
};
let chunks = chunk_transcript(&transcript);
```

（清稿補回標點後,chunk_transcript 的句界切分才會準 — 這就是斷句修復的主路徑。）

- [ ] **Step 4: cargo test + 編譯綠,手動本地驗一次**

Run: `cargo run --manifest-path server-rs/Cargo.toml` 後

```bash
curl -s localhost:1334/api/visualize -H 'content-type: application/json' -d '{
  "transcript": "嗯嗯那個那個今天就是說我們要討論一下那個線上預約系統對對對客戶現在用紙本嘛常常就是重複預約這樣子然後然後我們報季繳方案",
  "room": "cleanup-test"
}' | head -c 600
```

Expected: 卡片文字是「線上預約系統」「紙本重複預約」「季繳方案」之類乾淨短語,無「嗯」「那個」「就是說」「對對對」。

- [ ] **Step 5: Commit** `feat(api): stage-1 transcript cleanup before agent on agent/voice/visualize`

### Task 5: board-agent prompt 硬化（stage-2 防線）

**Files:**
- Modify: `prompts/board-agent.md`（content 規則區,約 line 38-44）

- [ ] **Step 1: content 規則加一條**

```
- 輸入是會議逐字稿,可能殘留口語贅字(嗯/那個/就是說/對對對)或辨識錯字:擷取「語意」寫成卡片,**絕不把贅字、語助詞、重複詞抄進卡片文字**;一句話只承載一個重點,寧可少建卡也不建雜訊卡。
```

- [ ] **Step 2: cargo test（agent 既有 parse 測試不受影響）+ Commit** `feat(prompts): board-agent never copies filler words into cards`

- [ ] **Step 3: 收 Phase 1 — push + PR + auto-merge**

```bash
git push -u origin feat/transcript-cleanup
gh pr create --fill && gh pr merge --auto --squash
```

---

## Phase 2 — 排版引擎：消除互疊（branch `feat/layout-overlap`）

### Task 6: layouts 改用卡片實際尺寸（col_positions 先行）

**Files:**
- Modify: `server-rs/src/layout.rs:105-128`

- [ ] **Step 1: 失敗測試 — 大卡片不互疊**

```rust
#[cfg(test)]
fn mk(id: &str, color: &str, w: f64, h: f64) -> Card {
    Card { id: id.into(), color: color.into(), x: 0.0, y: 0.0, w, h, frame_id: None, owner: None }
}
#[cfg(test)]
fn rects_disjoint(cards: &[Card], pos: &Pos) -> bool {
    let r: Vec<(f64, f64, f64, f64)> = cards
        .iter()
        .map(|c| { let (x, y) = pos[&c.id]; (x, y, c.w, c.h) })
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
```

- [ ] **Step 2: 跑測試確認 FAIL**（卡 b 高 420 蓋住卡 c）

- [ ] **Step 3: 實作 — 欄寬取該欄最大卡寬、列高累計實際高**

```rust
fn col_positions(cards: &[Card], ox: f64, oy: f64, sp: f64) -> Pos {
    let mut pos = Pos::new();
    let mut sorted: Vec<&Card> = cards.iter().collect();
    sorted.sort_by(|a, b| {
        column_of(&a.color)
            .cmp(&column_of(&b.color))
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    // 每欄寬度 = 該欄最大卡寬;欄 x 為前面所有欄寬累計
    let mut col_w: HashMap<usize, f64> = HashMap::new();
    for c in &sorted {
        let e = col_w.entry(column_of(&c.color)).or_insert(CARD_W);
        *e = e.max(c.w);
    }
    let mut col_x: HashMap<usize, f64> = HashMap::new();
    let mut x = ox;
    let mut cols: Vec<usize> = col_w.keys().copied().collect();
    cols.sort();
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
```

- [ ] **Step 4: cargo test 綠 + Commit** `fix(layout): columns honor actual card sizes (no overlap on resized cards)`

### Task 7: tree_positions 改 tidy-tree（父置中於子,杜絕跨欄穿線）

**Files:**
- Modify: `server-rs/src/layout.rs:166-214`

- [ ] **Step 1: 失敗測試**

```rust
#[test]
fn tree_parents_centered_over_children_and_disjoint() {
    // root -> a,b,c ; a -> a1,a2 ; b -> b1
    let cards: Vec<Card> = ["root", "a", "b", "c", "a1", "a2", "b1"]
        .iter().map(|id| mk(id, "yellow", 200.0, 200.0)).collect();
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
    // a 必須水平置中在 a1、a2 之間
    let (ax, _) = pos["a"]; let (a1x, _) = pos["a1"]; let (a2x, _) = pos["a2"];
    assert!((ax - (a1x + a2x) / 2.0).abs() < 1.0, "parent centered");
    // 子節點必須緊鄰父節點下一層、同一父的子節點相鄰(不被 c 插隊)
    let (a1y, a2y) = (pos["a1"].1, pos["a2"].1);
    assert_eq!(a1y, a2y);
}
```

- [ ] **Step 2: 確認 FAIL**（現行逐層堆疊,父不置中）

- [ ] **Step 3: 實作 — spanning tree + 遞迴 slot 配置**

```rust
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
    // spanning tree:只保留 level 差 1 的邊;每個子節點只認第一個父
    let mut kids: HashMap<String, Vec<String>> = ids.iter().map(|i| (i.clone(), vec![])).collect();
    let mut has_parent: HashSet<String> = HashSet::new();
    let mut order = ids.clone();
    order.sort_by(|a, b| {
        level[a].cmp(&level[b]).then(
            by_id[a].x.partial_cmp(&by_id[b].x).unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    for p in &order {
        for ch in children.get(p).cloned().unwrap_or_default() {
            if level[&ch] == level[p] + 1 && !has_parent.contains(&ch) {
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
    // 座標:主軸 = level,交叉軸 = slot;間距用整批卡片最大尺寸保證不疊
    let max_w = cards.iter().map(|c| c.w).fold(CARD_W, f64::max);
    let max_h = cards.iter().map(|c| c.h).fold(CARD_H, f64::max);
    let (slot_gap, level_gap) = if dir == "LR" {
        (max_h + 40.0 * sp, max_w + 50.0 * sp)
    } else {
        (max_w + 50.0 * sp, max_h + 40.0 * sp)
    };
    for id in &ids {
        let lv = level[id] as f64;
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
```

- [ ] **Step 4: cargo test 綠（含既有測試）+ Commit** `feat(layout): tidy-tree layout — parents centered, subtrees grouped`

### Task 8: radial 最小半徑（mindmap 卡多不再擠成一圈疊住）

**Files:**
- Modify: `server-rs/src/layout.rs:216-343`（radial_positions 末段半徑計算）

- [ ] **Step 1: 失敗測試**

```rust
#[test]
fn radial_dense_level_grows_radius() {
    // 中心 + 14 個一階子節點:固定半徑必互疊
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
```

- [ ] **Step 2: 確認 FAIL**

- [ ] **Step 3: 實作 — 每層半徑取「等差環距」與「該層卡數所需周長」較大者**

把 `let ring = 200.0 + 40.0 * sp;` 之後的定位段改成:

```rust
    let ring = 200.0 + 40.0 * sp;
    let max_lv = level.values().copied().max().unwrap_or(0).max(0);
    // 每層所需半徑:卡片對角線當弦長,該層 n 張要塞進 2π
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
```

注意:同層卡片角度由 leaf-count 分配,不保證等距 — 半徑算完後若仍有同層相鄰角度過近,交給 Task 10 的 safety-net 收尾。

- [ ] **Step 4: cargo test 綠 + Commit** `fix(layout): radial rings grow with card count (mindmap no longer self-overlaps)`

### Task 9: quadrant 改 mini-grid（SWOT 每象限 2 欄網格）

**Files:**
- Modify: `server-rs/src/layout.rs:345-373`

- [ ] **Step 1: 失敗測試**

```rust
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
    // green 7 張要排成 2 欄(4+3),不是 7 張一直欄
    let xs: HashSet<i64> = (0..7).map(|i| pos[&format!("g{}", i)].0 as i64).collect();
    assert_eq!(xs.len(), 2);
}
```

- [ ] **Step 2: 確認 FAIL**

- [ ] **Step 3: 實作**

```rust
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
    // 每象限排成 2 欄網格;象限寬高取四象限最大,避免擠壓
    let quad_rows = |n: usize| (n + COLS - 1) / COLS;
    let nmax_rows = ["green", "yellow", "blue", "red"]
        .iter()
        .map(|k| quad_rows(g.get(*k).unwrap_or(&empty).len()))
        .max()
        .unwrap_or(1) as f64;
    let quad_w = COLS as f64 * (CARD_W + gx);
    let quad_h = nmax_rows * (CARD_H + gy);
    let mut place = |arr: &Vec<&Card>, x0: f64, y0: f64, pos: &mut Pos| {
        for (i, c) in arr.iter().enumerate() {
            let (col, row) = (i % COLS, i / COLS);
            pos.insert(
                c.id.clone(),
                (x0 + col as f64 * (CARD_W + gx), y0 + row as f64 * (CARD_H + gy)),
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
```

- [ ] **Step 4: cargo test 綠 + Commit** `feat(layout): swot quadrants use 2-col grids with uniform quadrant size`

### Task 10: collision safety-net（任何 layout 輸出後的最終防線）

**Files:**
- Modify: `server-rs/src/layout.rs`（`layout_positions()` 包一層）

- [ ] **Step 1: 失敗測試**

```rust
#[test]
fn safety_net_separates_any_residual_overlap() {
    let cards = vec![mk("a", "yellow", 200.0, 200.0), mk("b", "yellow", 200.0, 200.0)];
    let mut pos = Pos::new();
    pos.insert("a".into(), (0.0, 0.0));
    pos.insert("b".into(), (50.0, 30.0)); // 故意疊
    resolve_collisions(&cards, &mut pos, 1.0);
    assert!(rects_disjoint(&cards, &pos));
}
```

- [ ] **Step 2: 確認 FAIL（函式不存在)**

- [ ] **Step 3: 實作 — 由上而下掃描,疊到就往下推(穩定、不會震盪)**

```rust
/// 最終防線:layout 演算法之外的任何殘餘重疊(radial 角度過近、奇形怪狀的圖)
/// 由上而下掃,疊到的卡往下推。最多迭代 6 輪,每輪 O(n^2)(n = 單一 frame 卡數,夠小)。
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
```

`layout_positions()` 尾端呼叫:

```rust
fn layout_positions(...) -> Pos {
    let bt = board_type(type_key);
    let mut pos = match bt.layout { /* 原 match 不變 */ };
    resolve_collisions(cards, &mut pos, sp);
    pos
}
```

- [ ] **Step 4: cargo test 綠 + Commit** `feat(layout): collision safety-net pass after every board layout`

### Task 11: tidy() frame re-packing（frame 互疊的根治）

現況:frame 只在「建立時」排到最右邊,tidy 只放大尺寸不移位置 → frame 長大後蓋到鄰居。改成:每個 frame 先在原點排版量尺寸,再整批 re-pack(列式換行),卡片座標 = frame 新原點 + 相對位置。

**Files:**
- Modify: `server-rs/src/layout.rs:546-616`(`Frame`、`tidy()`)
- Modify: `server-rs/src/store.rs:45-…`(`apply_tidy` 寫回 frame x/y)
- Modify: `server-rs/src/apply.rs`、`server-rs/src/lib.rs`(tidy 呼叫點型別)

- [ ] **Step 1: 失敗測試**

```rust
#[test]
fn tidy_repacks_frames_no_overlap() {
    use serde_json::json;
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
    let r: Vec<(f64,f64,f64,f64)> = fr.iter().map(|f| (f.x, f.y, f.w, f.h)).collect();
    assert!(!(r[0].0 < r[1].0 + r[1].2 && r[1].0 < r[0].0 + r[0].2
        && r[0].1 < r[1].1 + r[1].3 && r[1].1 < r[0].1 + r[0].3));
    // 每張卡要落在自己 frame 的矩形內
    for (id, x, y) in &pos {
        let f = if id.starts_with('c') { &fr[0] } else { &fr[1] };
        assert!(*x >= f.x && *y >= f.y && *x + 200.0 <= f.x + f.w + 1.0 && *y + 200.0 <= f.y + f.h + 1.0,
            "card {} ({},{}) inside frame", id, x, y);
    }
}
```

（fr 的順序按 packing 順序;測試裡用 id 對回。）

- [ ] **Step 2: 確認 FAIL**

- [ ] **Step 3: 實作**

```rust
pub struct FramePlace {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Returns (card positions, frame placements). Frameless boards => one whole-board layout.
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
        laid.push(Laid { frame: Frame { id: f.id.clone(), typ: f.typ.clone(), x: f.x, y: f.y }, rel, cards: fcards, w, h });
    }
    // pass 2:依目前閱讀順序(y 再 x)re-pack 成列,超寬換行 → frame 永不互疊
    laid.sort_by(|a, b| {
        a.frame
            .y
            .partial_cmp(&b.frame.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.frame.x.partial_cmp(&b.frame.x).unwrap_or(std::cmp::Ordering::Equal))
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
        out_frames.push(FramePlace { id: l.frame.id.clone(), x: cx, y: cy, w: l.w, h: l.h });
        cx += l.w + FRAME_GAP;
        row_h = row_h.max(l.h);
    }
    (out_pos, out_frames)
}
```

（空 frame 也參與 packing — `laid` 不再跳過 `fcards.is_empty()`,空的給最小尺寸 440x300。）

- [ ] **Step 4: 改 store::apply_tidy 同時寫回 frame x/y/w/h**

```rust
pub fn apply_tidy(
    room: &Room,
    positions: &[(String, f64, f64)],
    frames_out: &[crate::layout::FramePlace],
) {
    // shapes 迴圈不變;frames 迴圈:
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
```

呼叫點(apply.rs `tidy_board`、lib.rs `/api/rooms/:room/tidy`)型別跟著改,編譯器會抓全。

- [ ] **Step 5: cargo test 全綠 + Commit** `feat(layout): tidy re-packs frames — frames never overlap after growth`

### Task 12: 全板型重疊回歸測試

**Files:**
- Modify: `server-rs/src/layout.rs`(tests module)

- [ ] **Step 1: 加總測試 — 10 種板型 x 鏈狀連線 x 混合尺寸,全部不疊**

```rust
#[test]
fn all_board_types_produce_disjoint_layouts() {
    for t in ["meeting", "orgchart", "flow", "architecture", "mindmap",
              "kanban", "swot", "timeline", "fishbone", "gantt"] {
        let colors = ["yellow", "green", "blue", "red"];
        let cards: Vec<Card> = (0..13)
            .map(|i| {
                let mut c = mk(&format!("n{}", i), colors[i % 4], 200.0, 200.0);
                if i == 5 { c.w = 380.0; c.h = 300.0; } // 一張使用者拉大的卡
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
```

- [ ] **Step 2: cargo test 全綠（哪型炸了就修哪型,safety-net 應接住大多數）**

- [ ] **Step 3: Commit + 收 Phase 2 PR**

```bash
git commit -m "test(layout): overlap regression suite across all 10 board types"
git push -u origin feat/layout-overlap
gh pr create --fill && gh pr merge --auto --squash
```

---

## Phase 3 — 文件 + 部署驗證（branch `docs/pipeline-and-layout`）

### Task 13: README + 使用手冊更新

**Files:**
- Modify: `README.md`(AI pipeline 段落 + prompts 清單)
- Modify: `docs/`(GitHub Pages 手冊,若有對應段落)

- [ ] **Step 1: README 加「兩段式 AI」說明**:stage-1 清稿(transcript-cleanup.md,可改 prompt 即時生效;body `cleanup:false` 可關)→ stage-2 board-agent;排版段落補 frame re-packing 與不重疊保證。

- [ ] **Step 2: Commit + PR + auto-merge** `docs: two-stage ai pipeline + non-overlapping layout guarantees`

### Task 14: 本地端到端驗證 → 部署 → 線上驗證

- [ ] **Step 1: 本地完整跑**:`npm run build:client && cargo run --manifest-path server-rs/Cargo.toml`,用 /api/visualize 餵髒口語稿(含嗯/那個/對對對/無標點長串),確認:(a) 卡片文字乾淨 (b) 回 JSON cards/frames 數合理。

- [ ] **Step 2: 讀回 room 狀態驗排版**:再 POST 一段不同主題稿觸發第二個 frame,打 `/api/rooms/:room/tidy`,以 export 或讀 yrs 狀態確認 frame 不疊。

- [ ] **Step 3: 等所有 PR auto-merge 進 main**(CI 綠),Render 自動部署;`curl https://mori-canvas.onrender.com/api/health` 確認新版上線。

- [ ] **Step 4: 對線上 /api/visualize 打同一段髒稿,確認行為一致**(注意 demo 限流 60/min)。

- [ ] **Step 5: 給 yazelin 的驗證清單**(依 feedback_per_change_verify_checklist):列出「真的改了什麼 → 在 https://mori-canvas.onrender.com/ 怎麼驗」。

---

## Self-Review 紀錄

- 涵蓋 spec 三項:贅字/斷句(Task 2-5)、卡/線互疊(Task 6-12,線疊主因是 naive tree 與 frame 不移位,由 tidy-tree + re-packing 根治)、各板型排列優化(Task 7-9 + 12)。
- 型別一致:`FramePlace` 在 Task 11 定義,Task 11 Step 4 的 apply_tidy 與測試同用;`scrub_fillers`/`cleanup_transcript` 名稱前後一致。
- 已知風險:`strip_think` 位置待執行時確認(在 agent.rs 或 lib.rs,必要時 pub);apply_tidy 簽名變更會被編譯器逐點抓出。
