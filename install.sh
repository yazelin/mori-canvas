#!/usr/bin/env bash
# Mori Canvas — Linux 一鍵安裝(免 Rust / 免 Node)。
#
#   curl -fsSL https://raw.githubusercontent.com/yazelin/mori-canvas/main/install.sh | bash
#
# 做的事:
#   1. 偵測架構,從 GitHub Releases 抓最新的 mori-canvas-server tar.gz
#   2. 解到   ~/.local/share/mori-canvas/<版本>/
#   3. symlink ~/.local/bin/mori-canvas-server 指過去(冪等,可重跑升級)
#   4. 印出啟動指令與必要的環境變數說明
#
# 沒有對應 release(或非 x86_64)時,會給「從源碼 cargo build」的指引後結束。

set -euo pipefail

REPO="yazelin/mori-canvas"
SHARE_DIR="${HOME}/.local/share/mori-canvas"
BIN_DIR="${HOME}/.local/bin"
BIN_LINK="${BIN_DIR}/mori-canvas-server"

say()  { printf '%s\n' "$*"; }
die()  { printf '%s\n' "$*" >&2; exit 1; }

fallback_build_from_source() {
  say ""
  say "改用從源碼 build(需要 Rust 與 Node.js 18+):"
  say "  git clone https://github.com/${REPO} && cd mori-canvas"
  say "  npm install && npm run build    # vite build 前端 → cargo build --release(前端內嵌進 binary)"
  say "  ./server-rs/target/release/mori-canvas-server"
  say ""
  say "或直接用 Docker(免 build):"
  say "  docker run -p 1334:1334 -v \"\$PWD/data:/app/.data\" -e GROQ_API_KEY=gsk_xxx ghcr.io/${REPO}"
  exit 1
}

# --- 0) 前置檢查 -------------------------------------------------------------
command -v curl >/dev/null 2>&1 || die "需要 curl(sudo apt install curl)"
command -v tar  >/dev/null 2>&1 || die "需要 tar"

os="$(uname -s)"
arch="$(uname -m)"
if [ "$os" != "Linux" ]; then
  say "這個腳本只支援 Linux(偵測到:${os})。"
  say "macOS / Windows 請直接到 Releases 下載對應的 server 包:"
  say "  https://github.com/${REPO}/releases"
  fallback_build_from_source
fi
if [ "$arch" != "x86_64" ]; then
  say "目前 Releases 只有 linux-x86_64 預編譯包(偵測到:${arch})。"
  fallback_build_from_source
fi

# --- 1) 找最新 release 的 linux-x86_64 資產 ----------------------------------
say "查詢最新 release ..."
api_json="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true)"
if [ -z "$api_json" ] || printf '%s' "$api_json" | grep -q '"message": *"Not Found"'; then
  say "找不到任何 GitHub Release(可能還沒發佈第一版)。"
  fallback_build_from_source
fi

tag="$(printf '%s' "$api_json" | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"tag_name": *"//; s/"$//')"
url="$(printf '%s' "$api_json" | grep -o '"browser_download_url": *"[^"]*linux-x86_64\.tar\.gz"' | head -1 | sed 's/.*"browser_download_url": *"//; s/"$//')"
if [ -z "$tag" ] || [ -z "$url" ]; then
  say "最新 release(${tag:-?})裡找不到 mori-canvas-server-*-linux-x86_64.tar.gz 資產。"
  fallback_build_from_source
fi

# --- 2) 下載 + 解壓到 ~/.local/share/mori-canvas/<tag>/ ----------------------
dest="${SHARE_DIR}/${tag}"
if [ -x "${dest}/mori-canvas-server" ]; then
  say "已安裝過 ${tag}(${dest}),略過下載。"
else
  say "下載 ${tag}:${url}"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  curl -fL --progress-bar -o "${tmp}/server.tar.gz" "$url"
  tar xzf "${tmp}/server.tar.gz" -C "$tmp"
  unpacked="$(find "$tmp" -maxdepth 2 -name mori-canvas-server -type f | head -1)"
  [ -n "$unpacked" ] || die "tar.gz 裡找不到 mori-canvas-server,資產格式可能變了,請回報 issue。"
  mkdir -p "$dest"
  cp "$unpacked" "${dest}/mori-canvas-server"
  chmod +x "${dest}/mori-canvas-server"
  readme="$(dirname "$unpacked")/README.txt"
  [ -f "$readme" ] && cp "$readme" "${dest}/README.txt"
fi

# --- 3) symlink ~/.local/bin/mori-canvas-server(冪等)------------------------
mkdir -p "$BIN_DIR"
ln -sfn "${dest}/mori-canvas-server" "$BIN_LINK"
say "已連結 ${BIN_LINK} -> ${dest}/mori-canvas-server"

# --- 4) 收尾說明 --------------------------------------------------------------
say ""
say "安裝完成(${tag})。啟動:"
say "  mori-canvas-server          # 預設 http://0.0.0.0:1334,瀏覽器開 http://localhost:1334/"
say ""
say "環境變數(可放執行目錄的 .env,或 export):"
say "  GROQ_API_KEY=gsk_xxx        # AI(逐字稿→便利貼)必要;已有 ~/.mori/config.json 則免"
say "  PORT=1334  BIND=0.0.0.0     # 監聽 port / 綁定位址(預設值)"
say "  LLM_LOCAL_ONLY=1            # 鎖本機模式:AI 只走本機 Ollama,封鎖雲端 STT 與訪客 BYO"
say ""
say "房間資料存在「執行目錄」的 .data/;區網 HTTPS(手機錄音)與 systemd 常駐見:"
say "  https://github.com/${REPO}#%E9%83%A8%E7%BD%B2--%E7%B5%A6%E5%88%A5%E4%BA%BA%E7%94%A8"
case ":$PATH:" in
  *":${BIN_DIR}:"*) ;;
  *) say ""; say "提醒:${BIN_DIR} 不在 PATH,請加進 shell 設定:"
     say "  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
esac
