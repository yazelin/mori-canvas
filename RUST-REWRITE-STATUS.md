# Rust rewrite — status: COMPLETE (full parity + Tauri)

Decision (you): rewrite the server in **Rust** → one core that ships as a **web-app
binary** + a **Tauri desktop** version; the React/Konva client is reused unchanged.

**Status: done.** The Rust version reaches **full parity** with the Node version and
adds the Tauri desktop app. The Node `server/` is kept as the reference (delete it once
you're happy diffing behaviour).

## ✅ Everything, verified

Foundation de-risked first: **`rust-sync-spike/`** proved yrs (Rust) interops with the
classic yjs JS client (the `@y/websocket-server` gotcha does **not** apply to yrs).
Main crate: **`server-rs/`** (`mori-canvas-server`, a lib + thin bin). Desktop:
**`src-tauri/`** (`mori-canvas-app`).

| Phase | What | Verified |
|---|---|---|
| P1 | Multi-room yrs sync + `.data/<room>.bin` persistence; 10 board types; 6 layouts (columns/tree/radial/quadrant/fishbone/gantt) frame-aware; endpoints rooms/tidy/end/meta/frames/export; frame-aware markdown export | ✅ JS A→B sync; org cards → /tidy → tree; export sections |
| P2 | Agent (intent classify, board plans, frames, lenient parse, card-edit) + LLM Groq→Ollama (reqwest) | ✅ vs real Groq: content→frame+4 cards+3 conns; "幫我排一下"→tidy; "只看亞澤的"→filter |
| P3 | STT: mori-ear / Groq Whisper / local whisper-server + ffmpeg silence-trim; /api/voice, /api/card, /api/transcribe | ✅ mori mode hits whisper-server; custom-silence trims→skip |
| P4 | **Client embedded via include_dir** → Rust binary serves client + /api + /sync on one port | ✅ runs from /tmp (no client/dist on disk); full app in browser → agent → cards |
| P5 | **Tauri desktop app** embeds the server, loads the webview at the loopback, self-registers as a mori-desktop body part | ✅ builds + launches; embedded server up; `~/.mori/body-parts/mori.canvas/manifest.json` written |
| + | `/api/summary` (LLM meeting note), **per-room** agent lock, **streaming Mori cursor** (draws one-by-one) | ✅ summary sections vs Groq; cursor moves + cards stream 0→1→2→3→4 |

## Run

**Web-app binary** (self-contained, sellable):
```bash
npm run build                                   # client/dist (embedded at compile time)
cargo build --manifest-path server-rs/Cargo.toml
./server-rs/target/debug/mori-canvas-server     # PORT env (default 1334); runs from anywhere
```

**Tauri desktop**:
```bash
npm run build
cargo build --manifest-path src-tauri/Cargo.toml      # or: cargo tauri build (full bundle)
./src-tauri/target/debug/mori-canvas-app              # opens window, embeds server on :8731
```

## Integration

- **mori-desktop**: the Tauri app **self-registers** `~/.mori/body-parts/mori.canvas/manifest.json`
  on launch (kind `standalone_app`, entrypoint = the binary) — so it appears in mori-desktop's
  body-parts list. (Currently points at the *debug* binary; a release/installed run updates it.)
  The standalone server binary also has a gated `local_service` register (`MORI_CANVAS_REGISTER=1`).
- **AgentOS**: `agentos-manifest.json` (repo root) — `agentos install ./agentos-manifest.json`.

## Notes / follow-ups (small)

- **Icons**: `src-tauri/icons/` are placeholders copied from the recorder — swap for real
  Mori Canvas icons before shipping. `cargo tauri build` (full bundle/installer) may want
  `.icns`/`.ico` too.
- **Build order**: `npm run build` must run before `cargo build` (the client is embedded
  via `include_dir` at compile time). Re-run it when the client changes.
- Node `server/` is unchanged — keep it for parity diffing, then remove.
