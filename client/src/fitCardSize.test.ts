// fitCardSize 邊界 assert — 一次性測試腳本(不進 bundle;App 只 import fitCardSize.ts)。
// 跑法:
//   npx tsc client/src/fitCardSize.ts client/src/fitCardSize.test.ts \
//     --outDir /tmp/fit-test --module commonjs --target es2020 \
//   && node /tmp/fit-test/fitCardSize.test.js
import { fitCardSize, MIN_CARD_H, MAX_CARD_H, BASE_FONT } from './fitCardSize'

let failed = 0
function assert(cond: boolean, msg: string) {
	if (cond) console.log('ok: ' + msg)
	else {
		failed++
		console.error('FAIL: ' + msg)
	}
}

// 1. 空字串 → 預設 200 高、預設字級
{
	const r = fitCardSize('', 200)
	assert(r.h === MIN_CARD_H && r.fontSize === BASE_FONT, '空字串 → 200 / 19')
}
// 2. 全空白視同空字串
{
	const r = fitCardSize('   \n  ', 200)
	assert(r.h === MIN_CARD_H && r.fontSize === BASE_FONT, '全空白 → 200 / 19')
}
// 3. 短句(AI 卡典型長度 ≤14 字)不變形
{
	const r = fitCardSize('確認報價單寄給客戶', 200)
	assert(r.h === MIN_CARD_H && r.fontSize === BASE_FONT, '短 CJK 句 → 200 / 19')
}
// 4. 中長文(60 字):增高但不縮字
{
	const r = fitCardSize('中長文的會議重點內容共六十個字'.repeat(4), 200)
	assert(r.fontSize === BASE_FONT && r.h > MIN_CARD_H && r.h <= 260, `中長文 → 字級不變、200 < h <= 260(實得 ${r.h}/${r.fontSize})`)
}
// 5. 長文:先縮字級再增高
{
	const r = fitCardSize('長'.repeat(200), 200)
	assert(r.fontSize < BASE_FONT && r.h <= MAX_CARD_H, `200 字長文 → 字級縮小且 h <= 460(實得 ${r.h}/${r.fontSize})`)
}
// 6. 極端長文:最小字級 + 高度封頂 460
{
	const r = fitCardSize('字'.repeat(1000), 200)
	assert(r.h === MAX_CARD_H && r.fontSize === 12.5, '1000 字 → 460 / 12.5')
}
// 7. 換行各佔一行(含空行)
{
	const a = fitCardSize('一\n二\n三', 200)
	const b = fitCardSize('一二三', 200)
	assert(a.h >= b.h, '含換行的文字不會比同字數單行更矮')
}
// 8. 拉丁長文也會走縮字級路徑且不超界
{
	const r = fitCardSize('hello world '.repeat(20), 200)
	assert(r.fontSize <= BASE_FONT && r.h >= MIN_CARD_H && r.h <= MAX_CARD_H, `拉丁長文 → 高度在 [200, 460](實得 ${r.h}/${r.fontSize})`)
}
// 9. 卡越寬,同樣文字需要的高度不會更高
{
	const narrow = fitCardSize('寬卡片可以放更多字所以行數較少'.repeat(4), 200)
	const wide = fitCardSize('寬卡片可以放更多字所以行數較少'.repeat(4), 320)
	assert(wide.h <= narrow.h, `寬卡 h(${wide.h}) <= 窄卡 h(${narrow.h})`)
}
// 10. 掃一輪長度:輸出永遠在合法範圍、字級只會是階梯值
{
	const legal = new Set([19, 16, 14, 12.5])
	let all = true
	for (let n = 0; n <= 600; n += 17) {
		const r = fitCardSize('測'.repeat(n), 200)
		if (r.h < MIN_CARD_H || r.h > MAX_CARD_H || !legal.has(r.fontSize)) all = false
	}
	assert(all, '0..600 字掃描:h 永遠在 [200,460]、字級皆為階梯值')
}

if (failed) throw new Error(`fitCardSize: ${failed} 個 assert 失敗`)
console.log('fitCardSize: 全部 assert 通過')
