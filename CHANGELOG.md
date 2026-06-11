# Changelog

本檔記錄 Mori Canvas 的版本變更。格式參考 [Keep a Changelog](https://keepachangelog.com/)。

## v0.1.0 — 首個公開版本

第一個正式發行。**講話 / 貼逐字稿 → AI 自動整理成便利貼 + 關係圖 → 多人即時協作的會議白板**;自架、零授權成本(全 MIT)。後端純 Rust 單一 binary,前端 React + Konva 內嵌進 binary,前端 yjs CRDT 即時同步。

### 核心

- **兩段式 AI(清稿 → 畫卡)**:逐字稿先清贅字 / 補標點 / 重斷句(規則層 + LLM)再進畫卡 agent,贅字冗詞、斷錯句不會被抄進卡片。
- **AI 整理 + 累積合併**:逐字稿 → Groq `gpt-oss-120b`(本機 Ollama `qwen3` 後備)→ 便利貼(按 kind 上色)+ 關係連線;op-based 改寫 / 合併 / 刪除既有卡。
- **連續會議記錄(hands-free)**:VAD 靜音自動斷句;即時音量條、無聲提醒、辨識中狀態、段落失敗自動重試 + 手動重送、限流 429 退避續傳、手機螢幕喚醒鎖 + 中斷恢復、單聲道 48kbps 省流量。
- **語音指令(intent 判斷)**:分辨「會議內容」vs「指令」,排版 / 篩選 / 指派 / 改類型 / 加標籤 / 改寫 / 搬卡 / 連線 / 開命名區 直接執行;AI 從你的話認出是哪一張既有卡(內容 / 順序 / 編號)。
- **板型 × 自動排版(保證不互疊)**:10 種板型(會議 / 組織 / 流程 / 架構 / 心智圖 / 看板 / SWOT / 時間軸 / 魚骨 / 甘特),6 種排版,frame-aware;tidy-tree 父置中、心智圖環半徑隨卡數撐大、SWOT 網格、魚骨經典形 + 碰撞防護 + 圖框整批重排。

### 協作 / 分享

- **多人即時協作**:yrs(Rust)sync server,跟 yjs JS client 互通;可見的 Mori 游標 + 真人游標、speaker attribution。
- **唯讀分享連結 + 房主鎖板**:`?view=1` 唯讀、房主鎖定白板,server 在 ws 層 enforce(不是純 UI 隱藏)。
- **範例庫 + 互動導覽**:五個 persona 範例 + 講法示範、六步 spotlight 導覽、`?board=` 深連結、社群範本投稿通道。
- **匯出**:白板摘要 / HTML / MD / 整板 PNG(可複製剪貼簿)/ 可還原 .json;語言跟介面走。

### 介面 / 國際化

- **雙語介面(繁中 / English)**:react-i18next,瀏覽器語言自動偵測 + 設定頁切換;**AI 輸出語言跟著 `X-Lang` header 走**(卡片、摘要、匯出標題全部)。
- **深淺主題**:暖紙 / 森林夜,深色有專屬卡片色板;卡片類型小標。
- **第一次進來引導**:說明卡 + 互動導覽 + 範例庫,事後可重開。
- **多裝置 / 手機 / PWA**:responsive、雙指縮放、可裝成 PWA。

### STT(語音轉文字)三條路

- mori-ear / 雲端 Groq Whisper / 本機 whisper-server(設定頁切);送 STT 前 ffmpeg 靜音剪避免幻覺;繁中輸出程式硬轉(OpenCC)。

### 安全 / 隱私 / 部署

- **`ADMIN_TOKEN`**:鎖主機級設定與結束房間;未設時主機級欄位僅限**直連本機**(loopback 且無 `X-Forwarded-For`)修改 —— 反向代理 / PaaS 後面的訪客一律擋,不會被劫持。
- **`LLM_LOCAL_ONLY=1`**:開機鎖定本機模式,AI 只走本機 Ollama、雲端 STT 與訪客 BYO 端點全封鎖。
- **Bring Your Own AI**:訪客填自己的 OpenAI 相容 base/key/model,key 只存自己瀏覽器。
- **demo 站治理**:per-IP 限流(429 + Retry-After)、房間 TTL、`MAX_ROOMS`、房號隱私(清單只回數量)、常駐 DEMO 示範房(每小時重置)。
- **部署**:Render Blueprint(`render.yaml`)、Dockerfile + ghcr image、`install.sh` 一鍵安裝、Linux/macOS/Windows server binary、桌面安裝檔(.msi/.exe/.dmg/.AppImage/.deb)、`deploy/` systemd + nginx 範例、AgentOS body-part(`meeting.visualize`)。

### 文件

- 完整 GitHub Pages 文件站(首頁 / 操作手冊 / 範例教學 / 自架部署 / FAQ)、繁中 + 英文 README、部署比較表。

[完整提交歷史](https://github.com/yazelin/mori-canvas/commits/main)・[issues](https://github.com/yazelin/mori-canvas/issues)
