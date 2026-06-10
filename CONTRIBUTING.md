# 貢獻指南

謝謝你想讓 Mori Canvas 更好。三條路:

## 投稿範本

做了一張漂亮的板(SWOT / retro / OKR / 任何板型)?
匯出「畫板存檔(.json)」,照 [client/public/templates/README.md](client/public/templates/README.md)
的規範放進 `templates/` 開 PR —— 收錄後會出現在 app 內範例庫的「社群範本」區,
任何人一條 `?room=<新房號>&board=<範本id>` 連結就能載入你的板。

## 回報 bug / 提需求

開 [GitHub issue](https://github.com/yazelin/mori-canvas/issues):

- 寫清楚重現步驟、預期行為、實際行為。
- 部署方式(線上試玩 / 自架 / 桌面版)與瀏覽器版本有助於定位。
- 截圖或畫板存檔(.json,去敏後)很有幫助。

## 開 PR

- 短命 branch、小 PR,一個 PR 做一件事。
- 改前端跑 `npm run build:client`、改後端跑 `cargo test --manifest-path server-rs/Cargo.toml`,綠了再送。
- commit 訊息用英文 conventional commits(`feat:` / `fix:` / `docs:` …)。
- 範本投稿的細部規範見 [client/public/templates/README.md](client/public/templates/README.md)。

授權 MIT —— 送出 PR 即表示同意你的貢獻以 MIT 釋出。
