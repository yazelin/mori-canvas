# 社群範本投稿(templates/)

這個目錄收社群投稿的畫板範本。被收錄的範本會出現在 app 內「範例庫」的**社群範本**區,
任何人都能用一條深連結直接載入你的板:

```
https://<站>/?room=<新房號>&board=<範本id>
```

(app 內範例庫每個範本旁有「複製分享連結」按鈕,會自動產新房號。)

## 怎麼做一份範本

1. 在 [線上 app](https://mori-canvas.onrender.com/) 或自架站把板做好(用講的或手動排都行)。
2. 「匯出 → 畫板存檔(.json)」下載,這就是範本檔 —— 格式是 `mori-canvas/v1`,不用手寫。
3. 把檔名改成 `<範本id>.json` 放進本目錄,並在 `index.json` 的 `examples` 陣列加一筆條目。

`index.json` 條目 schema 與 `client/public/examples/index.json` 相同:

```json
{
  "id": "retro",
  "persona": "投稿者或角色",
  "title": "Sprint 回顧",
  "blurb": "一句話說明這份範本適合什麼場合。",
  "boards": ["看板"],
  "sampleUtterances": ["(選填)照著講就長出同款的示範句"]
}
```

## 投稿規範

- **範本 id**:只能用小寫英數與連字號(`[a-z0-9-]`),且**不可與內建 examples 的 id 重複**
  (深連結會先找 `examples/` 再找 `templates/`)。
- **格式**:`mori-canvas/v1`(畫板存檔匯出的原樣即可),至少要有 `shapes` 陣列;
  `transcript` 建議清空或留空陣列,別把真實會議逐字稿投進來。
- **卡片文字 ≤ 14 字**:便利貼是重點不是段落,超過會被擠到難讀。
- **顏色語意要對板型**:每種板型的黃/綠/藍/紅各有語意(例如會議白板 黃=主題、綠=待辦、
  藍=決議、紅=風險;SWOT 綠=優勢、黃=劣勢、藍=機會、紅=威脅)。
  完整對照表見 [範例教學的十種板型總表](https://yazelin.github.io/mori-canvas/examples.html#types)。
- **內容**:虛構或已去識別化的內容;不要放真實個資、客戶名稱、內部機密。

## PR 流程

1. Fork 本 repo,開 branch。
2. 加入 `client/public/templates/<範本id>.json`,並更新 `client/public/templates/index.json`。
3. 本機驗證:`npm run build:client` 要過,JSON 要是合法的 `mori-canvas/v1`。
4. 開 PR,說明這份範本的使用場景,附一張載入後的截圖。

## 會被列進範例庫的條件

- 符合上述規範(id、格式、字數、顏色語意、無敏感內容)。
- 板子載入後排版正常(maintainer 會實際載入看過)。
- 與既有範本有區隔(同場景的重複投稿會請你先合併或改良)。
