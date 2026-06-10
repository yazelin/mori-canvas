// 便利貼自動高度:依卡寬、字級、CJK 字寬估行數,算出文字放得下的卡高。
// 純函式(不碰 DOM / Konva),可直接單測;對應 App.tsx 卡片 Text 的排版參數:
// padding 20、lineHeight 1.25、預設 fontSize 19。
//
// 策略:字多時「先縮字級(19 → 16 → 14 → 12.5)再增高」— 每個字級有自己
// 願意長到的高度上限,超過就降一級字再算;最終 h 夾在 [200, 460]。
// AI 建卡(文字 ≤14 字)走 server 的 200x200,不經過這裡;這裡只服務
// 使用者手動輸入的長文字。

const PAD = 20 // 與卡片 Text 的 padding 一致
const LINE_HEIGHT = 1.25
export const MIN_CARD_H = 200
export const MAX_CARD_H = 460
export const BASE_FONT = 19

// 字級階梯與各自的高度上限(需要的高度超過上限 → 先降一級字)
const TIERS: { fontSize: number; maxH: number }[] = [
	{ fontSize: BASE_FONT, maxH: 260 },
	{ fontSize: 16, maxH: 320 },
	{ fontSize: 14, maxH: 400 },
	{ fontSize: 12.5, maxH: MAX_CARD_H },
]

// 估一個字的顯示寬:CJK(含全形標點/諺文/假名)約等寬 fontSize,其餘(拉丁/數字)約 0.55 倍
function charW(ch: string, fontSize: number): number {
	const c = ch.codePointAt(0) || 0
	const cjk =
		(c >= 0x2e80 && c <= 0x9fff) || // CJK 部首、假名、漢字(含 CJK 標點 0x3000-)
		(c >= 0xac00 && c <= 0xd7af) || // 諺文
		(c >= 0xf900 && c <= 0xfaff) || // 相容漢字
		(c >= 0xff00 && c <= 0xffef) || // 全形字元
		c >= 0x20000 // 擴展漢字區
	return cjk ? fontSize : fontSize * 0.55
}

// 這段文字在某字級、某可用寬度下需要幾行(逐字累積寬度的簡化換行)
function countLines(text: string, fontSize: number, innerW: number): number {
	let lines = 0
	for (const para of text.split('\n')) {
		let used = 0
		let n = 1
		for (const ch of para) {
			const w = charW(ch, fontSize)
			if (used > 0 && used + w > innerW) {
				n++
				used = w
			} else {
				used += w
			}
		}
		lines += para ? n : 1 // 空行也佔一行
	}
	return Math.max(1, lines)
}

/** 依文字量與卡寬,回傳這張卡該有的高度與字級。 */
export function fitCardSize(text: string, w: number): { h: number; fontSize: number } {
	const t = (text || '').trim() ? text : ''
	const innerW = Math.max(40, w - PAD * 2)
	if (!t) return { h: MIN_CARD_H, fontSize: BASE_FONT }
	for (const { fontSize, maxH } of TIERS) {
		const lines = countLines(t, fontSize, innerW)
		const need = Math.ceil(lines * fontSize * LINE_HEIGHT + PAD * 2)
		if (need <= maxH) return { h: Math.max(MIN_CARD_H, need), fontSize }
	}
	// 連最小字級都放不下:卡在最大高度(極端長文,文字可能略超出卡面)
	return { h: MAX_CARD_H, fontSize: TIERS[TIERS.length - 1].fontSize }
}
