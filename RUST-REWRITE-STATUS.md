# Rust rewrite — status (for when you're back)

Decision (you): rewrite the server in **Rust** → one core that ships as a **web-app
binary** + a **Tauri desktop** version; the React/Konva client is reused unchanged.
The **Node `server/` stays as the working app + parity reference** until Rust reaches
full parity.

## ✅ Done + verified (P1–P4) — a complete standalone Rust web app

Everything lives in **`server-rs/`** (crate `mori-canvas-server`). Foundation was
de-risked first: **`rust-sync-spike/`** proved yrs (Rust) interops with the classic
yjs JS client (the `@y/websocket-server` gotcha does **not** apply to yrs).

| Phase | What | Module | Verified |
|---|---|---|---|
| P1a | Multi-room yrs sync + debounced `.data/<room>.bin` persistence + ws | `sync.rs`, `yval.rs` | ✅ JS client A→B sync through Rust; snapshot written |
| P1  | 10 board types + 6 layouts (columns/tree/radial/quadrant/fishbone/gantt) frame-aware + all non-AI endpoints + frame-aware markdown export | `board_types.rs`, `layout.rs`, `store.rs`, `main.rs` | ✅ org cards → POST /tidy → tree layout; export sections correct |
| P2  | Agent (intent classify, board plans, frames, lenient parse, card-edit) + LLM cascade (Groq→Ollama via reqwest) | `agent.rs`, `llm.rs`, `apply.rs` | ✅ vs **real Groq**: content→frame+4 cards+3 conns; "幫我排一下"→tidy; "只看亞澤的"→filter |
| P3  | STT: mori-ear delegate / Groq Whisper / local whisper-server + ffmpeg silence-trim; /api/voice, /api/card, /api/transcribe | `stt.rs` | ✅ mori mode hits whisper-server; custom-silence trims→skip |
| P4  | **Rust binary serves the built client + /api + /sync on one port** (no Node, no Vite) | `main.rs` (warp::fs) | ✅ **full app in a real browser on the Rust binary**: loads, syncs, agent→Groq → 4 cards |

### Run the Rust app
```bash
npm run build                       # build the client → client/dist
cargo build --manifest-path server-rs/Cargo.toml
./server-rs/target/debug/mori-canvas-server     # run from repo root; serves on :1334
# open http://127.0.0.1:1334/?room=test
# env: PORT, CLIENT_DIR (default client/dist), MORI_CANVAS_REGISTER=1 (self-register body part)
```

## ⏳ Remaining

### P5 — Tauri desktop version
Wrap the **same** React/Konva client in a Tauri webview with the `server-rs` core as the
backend → a single native binary. Reference: **`mori-meeting-recorder`** (Tauri/Rust app).
Plan:
1. `src-tauri/` Tauri project; tauri.conf points `build.devUrl` / `distDir` at `client/dist`.
2. On launch, start the `mori-canvas-server` warp app on a loopback port in a background
   task (or refactor `main.rs` into a `lib.rs` `run(port)` the Tauri app calls). The webview
   loads `http://127.0.0.1:<port>/`.
3. mori-desktop **BodyManifest** self-register — already coded (`maybe_register_body_part`,
   gated by `MORI_CANVAS_REGISTER=1` so it never writes shared `~/.mori/` autonomously).
   In the Tauri build, call it unconditionally (kind: `standalone_app`, `entrypoints.app` =
   the binary), like the recorder's `manifest.rs`.

### Integration manifests
- **AgentOS**: `agentos-manifest.json` (repo root) is **written** — AppManifest v2,
  `kind: body-part`, `consumes transcribe.local` / `provides meeting.visualize`.
  Install with `agentos install ./agentos-manifest.json`.
- **mori-desktop**: self-register coded but **OFF by default** (I didn't autonomously
  write to your shared `~/.mori/body-parts/`). Enable per-run with `MORI_CANVAS_REGISTER=1`,
  or wire it on for the Tauri build.

### Polish / known gaps vs the Node version
- **Streaming Mori cursor**: Rust `apply_plan` writes cards in one batch (they appear
  together); the Node version streams them one-by-one with the live Mori cursor. Cosmetic;
  port later by adding the awareness-cursor writes + per-sticky sleep.
- **Awareness/cursors**: yrs-warp's BroadcastGroup relays awareness, so human/Mori cursors
  still work; only the *server-driven* streaming cursor during agent draw is not ported.
- **Per-room agent lock**: Rust uses one global `AGENT_LOCK` (serializes all agent turns)
  vs Node's per-room lock. Fine for one meeting host; make it per-room if needed.
- **/api/summary**: not yet ported (the one-page meeting-note LLM endpoint). Small.
- The Node `server/` is unchanged and remains the reference; diff behaviour against it.
