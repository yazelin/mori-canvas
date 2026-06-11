<p align="center"><img src="assets/logo.png" width="116" alt="Mori Canvas"></p>

# Mori Canvas

> 語言:**繁體中文** ・ [English](README.en.md)

**講話 / 貼逐字稿 → AI 自動整理成便利貼 + 關係圖 → 多人即時協作的會議白板。** 自架、零授權成本(全 MIT)。

**[立即試玩](https://mori-canvas.onrender.com/) ・ [先看示範板](https://mori-canvas.onrender.com/?room=DEMO) ・ [GitHub](https://github.com/yazelin/mori-canvas) ・ [請我喝咖啡](https://buymeacoffee.com/yazelin)**

> 已上線、持續開發中。歡迎試玩(免費示範站偶有 bug、會休眠),更歡迎 fork / 自架回去自己玩。覺得好用可以[請我喝杯咖啡](https://buymeacoffee.com/yazelin)。

**完整文件站(GitHub Pages):[yazelin.github.io/mori-canvas](https://yazelin.github.io/mori-canvas/)** —— [首頁](https://yazelin.github.io/mori-canvas/) ・ [操作手冊](https://yazelin.github.io/mori-canvas/guide.html) ・ [範例教學](https://yazelin.github.io/mori-canvas/examples.html) ・ [自架部署](https://yazelin.github.io/mori-canvas/selfhost.html) ・ [常見問題](https://yazelin.github.io/mori-canvas/faq.html)

```
會議語音 ──STT(雲端 Groq / 本機 whisper)──▶ 逐字稿 ──清稿──▶ AI 畫卡(Groq gpt-oss-120b / 本機 qwen3)──▶ 便利貼+連線 ──yjs──▶ 多人 live 白板
                                                                                人也能在同一張板上 拖拉 / 改字 / 連線 / 刪除
```

![共筆白板](docs/hero.png)

上圖整張板是把一段會議逐字稿丟給 AI 後**自動長出來**的:便利貼按性質上色(主題=黃、決議=藍、待辦=綠、風險=紅),箭頭表示關係。

不知道從哪開始?開 app 後右上「範例」一鍵載入五個 persona 範例板(會議記錄 / 工程人 / 產品經理 / 行銷人 / 顧問),每個都附「開會這樣講就會長出這張板」的講法示範。

---

## 怎麼運作(先看這個就懂要裝什麼)

**client–server 架構,所有重活(STT、AI、整理、白板狀態)都跑在「主機」那一台**,其他人只用瀏覽器連進來、零安裝。

![單機獨立執行 — 自己的電腦跑](docs/arch-local.png)

![上傳 Render 雲端執行](docs/arch-render.png)

### 從「你講一句話」到「白板動作」的完整流程

1. **瀏覽器**:麥克風錄音,偵測靜音自動切一段 → 音檔 POST 給主機(`/api/voice`)。
2. **主機 · STT**:把音檔轉成文字(mori-ear / 雲端 Groq Whisper / 本機 whisper-server,設定頁切)。
3. **主機 · 清稿(stage 1)**:STT 原文常有贅字(嗯/那個/對對對)、斷錯句、錯字。先過規則層(收斂重複字詞、刪段首語助詞),再用 LLM 做最小幅度清稿(`prompts/transcript-cleanup.md`:修錯字、補標點重斷句、刪冗詞、不增減語意);LLM 失敗就用規則層結果,**清稿永遠不擋住建卡**。太短的輸入(<10 字,多半是指令)只過規則層;API 可帶 `"cleanup": false` 跳過。
4. **主機 · AI 畫卡(stage 2)**:把「清好的文字」+「目前整張白板(每張卡的類型/負責人/標籤,帶索引)」丟給 LLM,它先判斷這句是**指令(command)**還是**會議內容(content)**,回一個 JSON。
5. **主機 · 驗證 + 執行**(寫死的規則,不是 AI):檢查 JSON 合法(索引在範圍、動作有效)。指令 → 直接執行(排版/篩選/指派/改類型/加標籤/改寫/搬卡/連線);內容 → 整理成便利貼,或改/併/刪既有卡。
6. **同步回所有人**:白板變動透過 yjs(websocket)即時廣播給每台瀏覽器;只影響「自己畫面」的指令(像篩選)回傳給講話的那台套用。

判斷指令靠的是 **LLM 的理解,不是關鍵字比對**:「交給阿明做」=指派、「改寫成線上掛號」=改文字、「改成風險」=改類型。貼現成逐字稿走同一條路,只是跳過第 2 步 STT。

- **AI**:Groq `gpt-oss-120b`(本機 Ollama `qwen3` 後備),key/model 讀共用 `~/.mori/config.json`。
- **STT**:三條路(設定頁切)—— mori-ear / 雲端 Groq Whisper / 本機 whisper-server;繁中輸出一律程式硬轉(OpenCC),不靠模型自律。
- **即時同步**:自寫 yrs(Rust)sync server,跟 yjs JS client 互通,不靠任何雲服務。
- **獨立可跑**(不依賴 mori 生態),但可選擇性裝進 AgentOS 當 body-part(`meeting.visualize` http-service)。跟 mori 的關聯=`mori-ear` CLI + 共用 config;其餘是獨立 FOSS app。

---

## 需要裝什麼

### A. 主機(跑整套的那台)

| 要件 | 用途 | 必要性 |
|---|---|---|
| **Rust**(`cargo`) | 跑 server —— 一顆 binary,內嵌前端 + API + 即時同步 | 自架原始碼時必要(用 Docker/預編譯包則免) |
| **Node.js 18+** + npm | **只用來 build 前端**(`npm install` + `npm run build`);跑的時候不需要 | 同上 |
| **Groq API key**(`~/.mori/config.json` 的 `providers.groq.api_key`、或 `.env` / 環境變數 `GROQ_API_KEY`) | 「逐字稿 → 便利貼」的 AI | agent 必要(或改用 BYO / 本機 Ollama) |
| **mori-ear** CLI + whisper,或本機 whisper-server,或雲端 Groq Whisper | 「錄音 → 文字」 | 只有要語音才需要;打字/貼逐字稿免 |
| Ollama + `qwen3`(`ollama serve`) | Groq 連不到時的本機後備 | 選用 |

> **只想打字不想語音?** Node + `npm install` + Groq key 就能跑(打字 / 貼逐字稿 → 便利貼)。

### B. 其他人(連進來協作)

**什麼都不用裝。** 同一區網(或拿到公開網址)+ 一個現代瀏覽器,開主機給的網址即可;要錄音就允許麥克風。他們錄的音傳到**主機**用**主機的** STT + AI 處理,不耗他們自己任何資源。

---

## 跑起來

後端是 Rust(`server-rs/`),一顆 binary 把前端 + API + 同步全包(前端 `client/dist` 用 `include_dir` 內嵌,所以可從任意目錄跑)。也有 Tauri 桌面版(`src-tauri/`)。

```bash
npm install
npm run build          # = vite build(前端)→ cargo build --release(Rust binary,內嵌前端)
```

### 1) 本機自己玩

```bash
npm run dev            # 編 + 跑 debug binary;預設 http://0.0.0.0:1334
```
開 `http://localhost:1334/?room=meet`。

### 2) 區網版(多裝置 / 手機也能錄音)

手機要用麥克風就得 **HTTPS**(瀏覽器規定)。Rust server 自己 serve HTTPS(`HTTPS=1` + `certs/`):

```bash
npm run setup          # = bash setup.sh:偵測本機 IP + 產含該 IP 的自簽憑證(放 certs/,已 gitignore);IP 變了重跑
npm run build          # 第一次 / 改過前端後
npm run start:lan      # = HTTPS=1 PORT=5174 BIND=0.0.0.0 ./server-rs/target/release/mori-canvas-server
```
大家連 `https://你的區網IP:5174/?room=meet`(注意 https)。每台第一次會跳自簽憑證警告 → 繼續前往即可(一次接受涵蓋頁面 + 同步 + 錄音)。對外無鑑權,內網信任場合 OK,**試完記得關**(`kill $(lsof -ti tcp:5174)`)。

### 語音(STT)三條路 —— ⚙ 設定裡切

| 模式 | STT 怎麼來 | 適合 |
|---|---|---|
| **Mori 處理** | 委派 `mori-ear`(它自己決定本機 whisper / Groq);僅在偵測到 mori-ear 時可選 | 你自己、已裝 Mori |
| **自訂 · 雲端** | Groq Whisper API,填自己的 Groq key | 客戶**零安裝**,首選 |
| **自訂 · 本機** | 打一台本機 **whisper-server**(`/inference`) | 不想資料出網 |

「自訂」模式送 STT 前會先做**靜音剪**(ffmpeg),避免 Whisper 對靜音產生幻覺(硬掰「(字幕製作:貝爾)」之類)。本機 whisper-server 安裝:`bash scripts/setup-whisper-linux.sh`(或 Windows 的 `scripts\setup-whisper-windows.ps1`),啟動在 `127.0.0.1:8089`,設定頁填 `http://127.0.0.1:8089/inference`(留空自動偵測 `~/.mori/whisper-server.json`)。

---

## 部署 / 給別人用

**先決定:單人還是多人?** 桌面 App 是**一個人在自己電腦上用**(server 跑 loopback、不對外);要**多人連進同一張板**就走 server(Docker / install.sh / 源碼 / Render)。下表挑你的情況:

| 你的情況 | 用哪種 | 單人 / 多人 | 要裝什麼 | 資料在哪 | 現在可用? |
|---|---|---|---|---|---|
| 只想快速看看、給朋友玩 | [線上試玩](https://mori-canvas.onrender.com/)(Render demo) | 多人同房 | 什麼都不用,點連結 | 站長機器(會清) | ✅ |
| 一個人在自己電腦上用 | **桌面 App 安裝檔**(.msi/.exe/.dmg/.AppImage/.deb) | 單人 | 下載安裝檔雙擊 | 自己電腦 | ✅ |
| 團隊自架、最快 | **Docker 一行**(ghcr image) | 多人 | Docker | 你掛的 volume | ✅ |
| 團隊自架、免 Rust/Node | **`install.sh`**(Linux server binary) | 多人 | 一行 `curl \| bash` | 主機 `.data/` | ✅ |
| 任何平台、開發者 | **從源碼 build** | 多人 | Rust + Node | 主機 `.data/` | ✅ |
| 自己架一個線上版 | **Render Blueprint** | 多人 | GitHub 帳號 | Render 容器(會清) | ✅ |
| 已經在用 AgentOS | 裝成 **body-part** | 整合 | AgentOS | — | ✅ |

> **v0.1.0 已發行**:ghcr image、各平台預編譯 server binary、桌面安裝檔(.msi/.exe/.dmg/.AppImage/.deb/.rpm)都已掛上 [Releases](https://github.com/yazelin/mori-canvas/releases/tag/v0.1.0),上表全部直接可用。之後推新的 `v*` tag,CI 會自動更新這些產物。

### 1) 試玩(免裝,點連結就玩)

社群試玩版部署在 Render:[mori-canvas.onrender.com](https://mori-canvas.onrender.com/)。朋友點開掃 QR 就進。想花十秒看「長好的板」,直接開[示範板](https://mori-canvas.onrender.com/?room=DEMO)(每小時自動重置)。AI 走站長的 Groq key(有 per-IP 限流);想用自己的額度可在 ⚙ 設定 → BYO 填任何 OpenAI 相容的 base/key/model,或直接貼逐字稿。

**自己部署到 Render(GitHub-driven):** render.com → GitHub 登入 → New + → Blueprint → 選 repo(自動讀 `render.yaml`)→ Environment 填 `GROQ_API_KEY` 與 `ADMIN_TOKEN`(見下方安全)→ Deploy。之後每次 `git push` 自動重部署。免費方案閒置 15 分鐘休眠(首次再進等 ~30-60 秒冷啟動)。

### 2) 自己跑 server(給團隊、想自架)

**最快:Docker 一行**(image 已發佈到 ghcr,隨 `v*` tag 自動更新):
```bash
docker run -p 1334:1334 -v "$PWD/data:/app/.data" -e GROQ_API_KEY=gsk_xxx ghcr.io/yazelin/mori-canvas
```
白板資料持久化在 `./data`(沒掛 volume 容器一刪就沒)。

**Linux 一鍵安裝**(抓 GitHub Releases 預編譯 binary,免 Rust / 免 Node):
```bash
curl -fsSL https://raw.githubusercontent.com/yazelin/mori-canvas/main/install.sh | bash
mori-canvas-server                    # 預設 http://0.0.0.0:1334
```
裝到 `~/.local/share/mori-canvas/`、指令 symlink 在 `~/.local/bin/`;重跑即升級。macOS / Windows 到 [Releases](https://github.com/yazelin/mori-canvas/releases) 抓對應 server 包。

> 以上 Docker image、預編譯 binary 與桌面安裝檔都已隨 **v0.1.0** 掛上 [Releases](https://github.com/yazelin/mori-canvas/releases);之後推新 `v*` tag,CI 會自動更新。想跑未發佈的最新 main,走下面「從源碼 build」。

**從源碼 build:**
```bash
git clone https://github.com/yazelin/mori-canvas && cd mori-canvas
npm install && npm run build
./server-rs/target/release/mori-canvas-server   # 預設 http://0.0.0.0:1334
```
要常駐 + 正式 TLS,用 [`deploy/mori-canvas.service`](deploy/mori-canvas.service)(systemd)+ [`deploy/nginx.conf.example`](deploy/nginx.conf.example)(反向代理),照檔頭註解換路徑、env 填進 `/etc/mori-canvas.env`。

### 3) 桌面 App(雙擊就開)

GitHub Releases 有 `.msi`/`.exe`(Windows)、`.AppImage`/`.deb`(Linux),雙擊安裝開原生視窗。本機 build:`npm run tauri`(內嵌 server 跑 loopback:8731)。桌面版是單機用;**要多人開會用上面「自己跑 server」**(binds 0.0.0.0)。

### 4) 裝進 AgentOS

`meeting.visualize` 是 AgentOS **http-service**(`/api/visualize`:整段逐字稿 → 建板 + 匯出)。裝好 [AgentOS](https://github.com/yazelin/agentos) 後 `agentos install /path/to/mori-canvas/agentos-manifest.json --principal me`。桌面版啟動(或 `MORI_CANVAS_REGISTER=1`)會寫 `~/.mori/mori-canvas-server.json` 服務描述讓 AgentOS dispatch 進來。Standalone 行為不受影響。

---

## 安全 / 隱私(公開部署必看)

- **`ADMIN_TOKEN`(env)**:公開部署請設。設了之後 `POST /api/settings` 與「結束房間」要帶相符的 `X-Admin-Token` header,否則 401 —— 訪客就改不動 whisperUrl、處理模式、本機模式、主機 Groq key 這些主機級欄位。
- **未設 token 時**:主機級欄位只接受**直連本機**(loopback 且**無 `X-Forwarded-For`**)的修改;排列間距等個人偏好不受限。⚠️ Render / 同主機反向代理是用 loopback 連到 app,所以「帶 XFF = 經過 proxy = 不是本機管理員」—— 這類部署一定要設 `ADMIN_TOKEN`(沒設的話 host 欄位對所有外部訪客一律拒改,不會被劫持,但你自己也改不了,要靠 token)。
- **`LLM_LOCAL_ONLY=1`**:開機鎖定本機模式 —— AI 只走本機 Ollama、雲端 STT 與訪客 BYO 端點一律封鎖、設定頁/API 都關不掉(資料不出網)。
- **唯讀分享 + 房主鎖板**:分享面板可「複製唯讀連結」(`?view=1`,打開的人只能看);建房的第一個人是房主,可「鎖定白板」讓其他人變唯讀。兩者都是 **server 在 ws 層丟棄無權連線的寫入**,不是純 UI 隱藏。
- **BYO key 只存自己瀏覽器**:訪客在設定頁貼的 Groq key 走 BYO header、隨請求帶上,不會變成全 server 共用、訪客之間也蓋不掉。
- **demo 站治理**:`DEMO_RATE_PER_MIN`(per-IP 限流,超限回 429 + Retry-After)、`ROOM_TTL_HOURS`(閒置房自動清,demo 設 72h)、`MAX_ROOMS`(房數上限)、`PUBLIC_ROOM_LIST=0`(房間清單只回數量、不洩房號)。詳見 `.env.example`。

---

## 操作

- **開會(主要用法)**:左下「**● 開始會議記錄**」→ 連續收音,講一段、停頓一下就自動斷句送轉錄 + AI 整理上板,整場 hands-free。錄音中按鈕上有**即時音量條**;太久沒聲音會提醒檢查麥克風;切到別的 app / 熄屏被系統中斷會跳「**點此恢復錄音**」。也可「單次錄一段」或貼逐字稿「丟給 agent」。「**撤銷上輪 AI**」一鍵移除上一輪 AI 新增的卡與連線。
- **第一次進來**:有引導卡 + 六步互動導覽(spotlight 指向真實按鈕);之後從右上「?」可重看、重跑、或開**範例庫**。
- **板上互動**:雙擊空白新增、雙擊便利貼改字(長文字會**自動調高度與字級**)、拖拉移動、連線模式點兩張連線、選取後 Delete 刪除、**Ctrl+F 搜尋卡片**(命中平移鏡頭 + 高亮)、Ctrl+Z 復原(含圖框的刪/搬/改名)、空白拖曳平移、滾輪/雙指縮放、回正、清空。
- **分享 / QR**:工具列「分享 / QR」→ 設名字、顯示房號(短代碼)+ QR(手機掃了直接進)+ 連結 + 進行中房間;另有複製唯讀連結、鎖定白板、結束此房。
- **匯出**:**白板摘要**(AI 依板型整理成一頁紀錄)/ **HTML**(摘要 + 板圖 + 逐字稿,雙擊就能讀)/ **MD** / **整板 PNG**(主題色背景、可一鍵複製到剪貼簿);**畫板存檔(.json)** 可完整還原、傳給別人接著編。匯出語言跟介面語言走。
- **板型 × 自動排版**:工具列徽章切板型,共 **10 種**(會議 / 組織架構 / 流程 / 系統架構 / 心智圖 / 看板 / SWOT / 時間軸 / 魚骨 / 甘特)。板型(type+topic 存在 yjs)決定 AI 怎麼解讀卡片與連線、怎麼配色、怎麼排版。**排版保證卡片與圖框都不互疊**(樹狀=tidy-tree 父置中、心智圖環半徑隨卡數撐大、SWOT 網格、魚骨經典形;每次排版跑碰撞防護,「排好」時圖框整批重排)。
- **房間管理**:房間落地保留(`.data/<房號>.bin`),沒人連也在、重啟還原。`?room=DEMO` 是常駐示範房(每小時重置回種子);範例庫每份範本有深連結 `?room=<新房號>&board=<id>`,對方點開新房自動長出整張板。

API(同一個 port,本機 dev 預設 :1334;`/sync` 是 ws、其餘 HTTP):
```bash
curl -X POST localhost:1334/api/agent/meet -H 'Content-Type: application/json' \
  -d '{"transcript":"今天開會討論…"}'                # 逐字稿 → 板(一句一句)
curl localhost:1334/api/export/meet                  # 匯出 markdown(?lang=en 出英文)
curl -X POST localhost:1334/api/visualize -H 'Content-Type: application/json' \
  -d '{"transcript":"整場逐字稿…"}'                  # 一次到位:整段 → 建板 → 回 markdown/summary + 可繼續編輯的 url
# 帶自己的 AI(BYO):加 -H "X-LLM-Base: …" -H "X-LLM-Key: …" -H "X-LLM-Model: …"
# 指定 AI 輸出語言:加 -H "X-Lang: en"
```

---

## 介面語言 / Language

UI 支援**繁體中文(zh-TW)與 English**:第一次依瀏覽器語言自動偵測(`navigator.language` 是 `zh*` → 繁中,其餘 → English),⚙ 設定可手動切、記住選擇(`localStorage` 的 `wb-lang`)。**AI 輸出跟著語言走**:client 對每個 AI 請求帶 `X-Lang` header(`/api/summary`、`/api/export` 用 `?lang=`),`en` 時 server 在 prompt 附加英文輸出指令、跳過 OpenCC 繁化,匯出的板型名/區段標題/連線標題也轉英文;沒帶 header 的請求行為與舊版逐字相同(zh-TW 預設)。範例庫內容是「內容不是 UI」,維持繁中;專有名詞(Mori Canvas、Groq、Ollama…)兩語都不翻。

---

## 社群範本

範例庫(app 內右上「範例」)除了內建五個 persona 範例,也收社群投稿的範本(`client/public/templates/`)。想投自己的板看 [templates/README.md](client/public/templates/README.md);回報問題與 PR 規範見 [CONTRIBUTING.md](CONTRIBUTING.md)。

---

## 功能一覽

- **兩段式 AI(清稿 → 畫卡)**:逐字稿先清贅字/補標點/重斷句(`prompts/transcript-cleanup.md` 改了即生效)再進畫卡 agent,贅字冗詞、斷錯句不會被抄進卡片。
- **AI 整理 + 累積合併**:逐字稿 → Groq `gpt-oss-120b`(qwen3 後備)→ 便利貼(按 kind 上色)+ 關係連線;餵現有卡給 agent 只加新重點不重複,op-based 還能改寫/合併/刪除既有卡。
- **連續會議記錄 + 韌性**:VAD 靜音自動斷句、hands-free;即時音量條、無聲提醒、辨識中狀態、段落失敗自動重試 + 手動重送、限流 429 退避續傳、手機螢幕喚醒鎖 + 中斷恢復、單聲道 48kbps 省流量上傳。
- **語音指令(intent 判斷)**:agent 分辨「會議內容」vs「指令」,講「幫我排一下 / 只看亞澤的 / 把這張指給小明 / 改成決議 / 把 3 號連到 5 號 / 開三個區」直接執行,不用找按鈕。
- **語音會議主持**:命名區(zones)+ 搬卡(move)+ 連線(connect);AI 從你的話認出是哪一張卡(內容/順序/編號),改既有卡而非重複建。
- **負責人 + 標籤 + 篩選 + 搜尋**:agent 抽負責人(chip)與標籤(#tag),點 chip 篩選;Ctrl+F 搜尋卡片並平移高亮。
- **Speaker attribution + 可見的 Mori**:設名字後卡片標「誰提的」、游標顯示真名;agent 寫卡時 yjs awareness 廣播 Mori 游標,卡片串流冒出、畫完離開。
- **板型 × 自動排版(保證不互疊)**:10 種板型、6 種排版,frame-aware;tidy-tree / 自適應放射 / SWOT 網格 / 魚骨經典形 + 碰撞防護 + 圖框整批重排。
- **唯讀分享 + 房主鎖板**:`?view=1` 唯讀連結、房主鎖定白板,server ws 層 enforce。
- **範例庫 + 互動導覽**:五個 persona 範例 + 講法示範、六步 spotlight 導覽、`?board=` 深連結、社群範本投稿通道。
- **匯出**:白板摘要 / HTML / MD / 整板 PNG(可複製剪貼簿)/ 可還原 .json;語言跟介面走。
- **Bring Your Own AI**:訪客填自己的 OpenAI 相容 base/key/model,用自己額度。
- **雙語介面(zh-TW / English)**:react-i18next,自動偵測 + 設定頁切 + AI 輸出語言跟著走。
- **深淺主題**:☾/☀ 切換亮/暗色(暖紙 / 森林夜),深色有專屬卡片色板。
- **持久化 + 房間治理**:每房 `.data/<room>.bin` 重啟自動還原;TTL / MAX_ROOMS / 房號隱私 / 常駐 DEMO 房。
- **多裝置 / 手機 / PWA**:Rust 自帶 HTTPS 或部署 Render;手機可看可編可錄,responsive + 雙指縮放 + 可裝成 PWA。
- **硬化 / 部署**:`ADMIN_TOKEN`、`LLM_LOCAL_ONLY`、per-IP rate-limit、Dockerfile + ghcr + render.yaml + install.sh + `deploy/` systemd/nginx 範例。

![presence](docs/presence.png)

---

## 架構 / 檔案

後端 **Rust**(`server-rs/`,crate `mori-canvas-server`);前端 `client/` 內嵌進 binary。

| 部件 | 檔案 | 說明 |
|---|---|---|
| sync server | `server-rs/src/sync.rs` | `yrs` + `yrs-warp` 多房同步(跟 yjs JS client 互通)+ 持久化 `.data/<room>.bin`;唯讀/鎖板的 ws 層寫入過濾 |
| agent / LLM | `server-rs/src/agent.rs`, `llm.rs`, `apply.rs` | 逐字稿 → 意圖判斷 → board plan/指令;Groq(`gpt-oss-120b`)→ Ollama(`qwen3`);BYO;X-Lang 輸出語言;串流 Mori 游標 |
| 清稿(stage 1) | `server-rs/src/cleanup.rs`, `prompts/transcript-cleanup.md` | 規則層(重複字詞/段首語助詞)+ LLM 最小幅度清稿;失敗 fallback,不擋建卡 |
| 排版 / 板型 | `server-rs/src/layout.rs`, `board_types.rs` | 6 種排版 + 10 種板型,frame-aware;碰撞防護 + 圖框 re-packing,卡片/圖框保證不互疊 |
| STT | `server-rs/src/stt.rs` | Mori(委派 `mori-ear`)/ 自訂(Groq Whisper / 本機 whisper-server + ffmpeg 靜音剪) |
| HTTP / 服務 | `server-rs/src/lib.rs`(`serve`) | warp:`/api/*` + `/sync` ws + 內嵌前端;限流 / ADMIN_TOKEN / 房間治理 / DEMO 種子;`HTTPS=1`+`certs/` 自帶 TLS;`BIND`/`PORT` 可調 |
| client | `client/src/App.tsx`, `i18n.ts`, `locales/*.json`, `fitCardSize.ts` | yjs + WebsocketProvider 同步 → react-konva 渲染;全部互動 + 錄音/agent 面板 + 範例庫 + 導覽 + 搜尋 + i18n |
| 桌面版 | `src-tauri/` | Tauri 2:內嵌 server + webview;啟動自我登記 mori-desktop body part |

文件站(`docs/`,GitHub Pages):首頁 / 操作手冊 / 範例教學 / 自架部署 / FAQ 五頁。

---

## 踩過的雷(寫給下一棒)

1. **`@y/websocket-server`(yjs v3 官方 server)不能用 classic yjs client 寫** —— 它依賴 fork `@y/y`,client→server 寫噴 `store.getClock is not a function`。解法:用 `yrs` + `yrs-warp`(Rust)自寫 classic-yjs server(`sync.rs`),跟 yjs JS client 互通。
2. **非 ASCII 房名要 `decodeURIComponent`**:WS 路徑沒 decode、`/api/:room` 被 HTTP 路由自動 decode → 同名變兩個房,曾害「agent 說 6 張卻畫面空白」。
3. **手機錄音要 HTTPS**:`http://<區網IP>` 是不安全來源,瀏覽器擋 `getUserMedia` —— 所以才有區網版的自簽 HTTPS。
4. **agent JSON**:gpt-oss/qwen3 會夾 `<think>`/圍欄,要先剝再取外層 `{...}`;qwen3 記得 `think:false`;connector 用 `{from,to}` 別用 `[[a,b]]`。
5. **反向代理把訪客當本機管理員**:Render / 同主機 nginx 是用 loopback 連到 app,只看 socket 是不是 loopback 會把全世界當管理員。判斷「真本機」要 loopback **且無 `X-Forwarded-For`**(帶 XFF = 經過 proxy)。任何「loopback=信任」的判斷在 PaaS / 反代後面都要再要求無 XFF。
6. **warp filter 鏈加深要 `#![recursion_limit = "256"]`**(server-rs 的 lib + bin、以及內嵌同一套路由的 `src-tauri` 都要;桌面 build 只在打 tag 時跑,漏掉會在第一次發 release 才爆),否則巨型嵌套型別爆遞迴上限。
7. **Docker build 漏 COPY 種子檔**:DEMO 房種子是 `include_str!` 讀 `client/public/examples`,server build stage 漏 COPY 會讓每次 Render 部署 build fail。
8. **i18n 輸出語言**:只在 system prompt 尾端附英文指令會被前面大量中文指示稀釋(模型只翻標題、卡片仍中文);要強化措辭 + 在 user message 開頭也壓一行英文指令。
9. **server 重啟丟資料**:debounce 寫盤,要 SIGTERM/SIGINT flush(已做)。

---

## 現況 / 路線

- **現況**:已公開([yazelin/mori-canvas](https://github.com/yazelin/mori-canvas),MIT),Render 社群試玩版 + 自架皆可。後端純 Rust 單一 binary,房間持久化在 `.data/<房號>.bin`(Render 免費方案休眠/重部署會清掉,想留就「下載畫板檔」)。
- **待辦與想法**:走 [GitHub issues](https://github.com/yazelin/mori-canvas/issues);已知小缺口、未來方向都在那裡追。
- **授權**:後端 Rust(yrs / yrs-warp / warp / reqwest / tokio,MIT/Apache-2.0)、前端(yjs / konva / react-konva / react / vite,全 MIT)—— 可閉源、可賣,沒有 tldraw 那顆 production license。語音三條路,**不裝 mori-ear 也能跑**(填 Groq key 走雲端即可)。
