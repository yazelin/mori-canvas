import { useEffect, useMemo, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { Stage, Layer, Group, Rect, Text, Arrow, Circle } from 'react-konva'
import * as Y from 'yjs'
import { WebsocketProvider } from 'y-websocket'
import QRCode from 'qrcode'
import { useTranslation, Trans } from 'react-i18next'
import { apiLang, setLang } from './i18n'
import { fitCardSize, BASE_FONT } from './fitCardSize'

type Sticky = {
	id: string
	x: number
	y: number
	w: number
	h: number
	text: string
	color: string
	drawnBy?: string
	owner?: string
	tags?: string[]
	frameId?: string // 屬於哪張圖框;無 = 自由卡
	note?: boolean // 備註卡:自動排列與 AI 都不動它
	type?: string // yjs 物件種別(sticky)
	fontSize?: number // 自動高度時可能縮的字級;無 = 預設 19
}
type Connector = { id: string; from: string; to: string }

const COLORS: Record<string, string> = {
	yellow: '#f7e3a6', // topic — soft pastel
	green: '#c4e3bc', // todo
	blue: '#bdd6f1', // decision
	red: '#f3c8be', // risk
	note: '#e7e0f5', // 備註 — annotation, addable to any diagram (pale lavender, off the 4 kinds)
}
// a darker tint of each, for the little "kind" accent dot on a card
const KIND_ACCENT: Record<string, string> = {
	yellow: '#b88a18',
	green: '#4e9a5c',
	blue: '#4a72b8',
	red: '#c46a61',
	note: '#7c6bb0',
}
// canvas (Konva) can't read CSS vars, so the frame / handle / connectors / labels get
// their own light+dark values here — keeps the board readable in dark mode.
const CANVAS_LIGHT = {
	frameFill: 'rgba(253,251,247,0.55)',
	frameStroke: 'rgba(28,26,23,0.1)',
	frameHeader: 'rgba(180,83,10,0.09)',
	frameTitle: 'rgba(28,26,23,0.72)',
	handle: 'rgba(28,26,23,0.16)',
	handleStroke: 'rgba(28,26,23,0.28)',
	conn: 'rgba(28,26,23,0.42)',
	frameShadow: 'rgba(28,26,23,0.16)',
}
const CANVAS_DARK = {
	frameFill: 'rgba(255,255,255,0.045)',
	frameStroke: 'rgba(236,231,220,0.16)',
	frameHeader: 'rgba(217,118,42,0.18)',
	frameTitle: 'rgba(236,231,220,0.8)',
	handle: 'rgba(236,231,220,0.14)',
	handleStroke: 'rgba(236,231,220,0.34)',
	conn: 'rgba(236,231,220,0.4)',
	frameShadow: 'rgba(0,0,0,0.45)',
}
const CANVAS_FONT = "'Noto Sans TC', 'PingFang TC', 'Microsoft JhengHei', system-ui, sans-serif"
// 深色主題的卡片色板:同色相、明度大幅壓低 — 亮紙卡在森林夜背景上像日光燈,刺眼(Konva 讀不到 CSS var)
const COLORS_DARK: Record<string, string> = {
	yellow: '#cfa84e',
	green: '#7fae74',
	blue: '#6f94bf',
	red: '#c08174',
	note: '#8d7eb5',
}
// 壓暗後的卡上,色點/類型小標用更深的同色系維持對比
const KIND_ACCENT_DARK: Record<string, string> = {
	yellow: '#6e5212',
	green: '#2c5e36',
	blue: '#27466e',
	red: '#7a3a32',
	note: '#473a6b',
}
// 卡面的墨色/雜項(卡始終是「紙」,兩主題的字都維持深色)
const CARD_LIGHT = {
	text: '#1f1c18',
	num: 'rgba(28,26,23,0.4)',
	chipBg: 'rgba(28,26,23,0.1)',
	chipText: 'rgba(28,26,23,0.6)',
	ownerBg: 'rgba(180,83,10,0.18)',
	ownerText: '#8a3f08',
	stroke: 'rgba(255,255,255,0.45)',
	selStroke: '#1c1a17',
}
const CARD_DARK = {
	text: '#171310',
	num: 'rgba(15,12,9,0.55)',
	chipBg: 'rgba(0,0,0,0.2)',
	chipText: 'rgba(15,12,9,0.75)',
	ownerBg: 'rgba(46,21,4,0.32)',
	ownerText: '#33180a',
	stroke: 'rgba(0,0,0,0.25)',
	selStroke: '#f0ead9',
}
// card kinds — labels live in the locale files (kind.<color>)
const KIND_KEYS = ['yellow', 'green', 'blue', 'red', 'note'] as const
const KIND_ORDER = ['yellow', 'green', 'blue', 'red'] as const
// board types (mirror of server/board-types.ts, for the picker + badge);
// labels/blurbs live in the locale files (boardType.<key>.label/.blurb)
const WB_TYPE_KEYS = ['meeting', 'orgchart', 'flow', 'architecture', 'mindmap', 'kanban', 'swot', 'timeline', 'fishbone', 'gantt'] as const
// 錄音中的即時音量條:自己輪詢 ref,只重繪這個小元件
function VolBars({ level }: { level: React.MutableRefObject<number> }) {
	const { t } = useTranslation()
	const [v, setV] = useState(0)
	useEffect(() => {
		const t = setInterval(() => setV(level.current), 150)
		return () => clearInterval(t)
	}, [level])
	const n = Math.min(5, Math.ceil(v * 90)) // SPEAK 門檻 0.018 約亮 2 格
	return (
		<span style={{ display: 'inline-flex', gap: 2, alignItems: 'flex-end', height: 14, marginLeft: 9 }} title={t('panel.volumeTitle')}>
			{[0, 1, 2, 3, 4].map((i) => (
				<span key={i} style={{ width: 3, height: 4 + i * 2.5, borderRadius: 1, background: i < n ? 'currentColor' : 'rgba(127,127,127,0.35)' }} />
			))}
		</span>
	)
}
// 互動導覽步驟:sel 選不到的步驟會自動略過(例如手機版沒有桌面匯出鈕);文案在 locale tour.<key>
const TOUR_STEPS: { sel: string; key: string }[] = [
	{ sel: '[data-tour="record"]', key: 'record' },
	{ sel: '[data-tour="paste"]', key: 'paste' },
	{ sel: '.toolstrip', key: 'toolstrip' },
	{ sel: '[data-tour="share"]', key: 'share' },
	{ sel: '[data-tour="export"]', key: 'export' },
	{ sel: '[data-tour="help"]', key: 'help' },
]
// hierarchical types use orthogonal "axis" connectors (elbow), routed by direction
const WB_TREE_DIR: Record<string, 'TB' | 'LR'> = { orgchart: 'TB', architecture: 'TB', flow: 'LR', timeline: 'LR' }
// elbow polyline from rect a -> rect b. TB routes via a shared horizontal mid-line
// (siblings share it so lines don't overlap); LR via a vertical mid-line.
function elbowPoints(a: Sticky, b: Sticky, dir: 'TB' | 'LR'): number[] {
	const acx = a.x + a.w / 2
	const bcx = b.x + b.w / 2
	const acy = a.y + a.h / 2
	const bcy = b.y + b.h / 2
	if (dir === 'TB') {
		const down = bcy >= acy
		const sy = down ? a.y + a.h : a.y
		const ey = down ? b.y : b.y + b.h
		const midY = (sy + ey) / 2
		return [acx, sy, acx, midY, bcx, midY, bcx, ey]
	}
	const right = bcx >= acx
	const sx = right ? a.x + a.w : a.x
	const ex = right ? b.x : b.x + b.w
	const midX = (sx + ex) / 2
	return [sx, acy, midX, acy, midX, bcy, ex, bcy]
}
// Same-origin: the API and the sync websocket both go through Vite's reverse
// proxy, so this works over http OR https (and behind a tunnel) with no hardcoded
// port. ws upgrades to wss automatically when the page is served over https.
const SYNC_HTTP = '' // relative -> /api/... on the current origin
// ?view=1 = 唯讀檢視連結(分享成品用):隱藏編輯 UI;真正的寫入封鎖在 server ws 層
const READ_ONLY = new URLSearchParams(location.search).get('view') === '1'
const SYNC_WS = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/sync`

// short, human-readable room code (no ambiguous chars). Doubles as the "房號".
const CODE_ALPHABET = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789'
function genCode(n = 4): string {
	let s = ''
	for (let i = 0; i < n; i++) s += CODE_ALPHABET[Math.floor(Math.random() * CODE_ALPHABET.length)]
	return s
}
// use ?room= if present; otherwise mint a fresh code and put it in the URL (shareable)
function resolveRoom(): string {
	const p = new URLSearchParams(location.search)
	let r = p.get('room')
	if (!r) {
		r = genCode()
		p.set('room', r)
		history.replaceState(null, '', location.pathname + '?' + p.toString())
	}
	return r
}
// ?board=<id> 深連結:進房後若房間是空的,自動載入該範本(examples/ 或 templates/)。
// id 只允許 [a-z0-9-],所以永遠只會指到我們自己站上的靜態 JSON,不可能打到外部網址。
function resolveBoardParam(): string {
	const b = new URLSearchParams(location.search).get('board') || ''
	return /^[a-z0-9-]+$/.test(b) ? b : ''
}

const ACCENT = '#b4530a'
// Konva perf: skip the "perfect draw" buffer-canvas pass on every node — none of our
// shapes need it (it only matters for translucent fill+stroke overlap), and it costs
// an extra offscreen draw per node per frame.
const PERF = { perfectDrawEnabled: false } as const
// lighten a #rrggbb toward white (for the soft top of the sticky gradient)
function lighten(hex: string, amt = 0.09): string {
	const m = /^#?([0-9a-f]{6})$/i.exec(hex)
	if (!m) return hex
	const n = parseInt(m[1], 16)
	const r = Math.min(255, ((n >> 16) & 255) + Math.round(255 * amt))
	const g = Math.min(255, ((n >> 8) & 255) + Math.round(255 * amt))
	const b = Math.min(255, (n & 255) + Math.round(255 * amt))
	return `rgb(${r},${g},${b})`
}

// crisp monochrome line icons (inherit the button's colour, so they theme automatically)
const Ico = ({ children, size = 17 }: { children: React.ReactNode; size?: number }) => (
	<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" style={{ display: 'block' }}>
		{children}
	</svg>
)

// where the center->target line exits a w×h rectangle centred at (cx,cy)
function edgePoint(cx: number, cy: number, hw: number, hh: number, tx: number, ty: number): [number, number] {
	const dx = tx - cx
	const dy = ty - cy
	if (dx === 0 && dy === 0) return [cx, cy]
	const s = Math.min(dx !== 0 ? hw / Math.abs(dx) : Infinity, dy !== 0 ? hh / Math.abs(dy) : Infinity)
	return [cx + dx * s, cy + dy * s]
}

export default function App() {
	const { t, i18n } = useTranslation()
	const uiLang = i18n.language === 'en' ? 'en' : 'zh-TW'
	// kind labels / board-type labels follow the UI language (used on the Konva canvas too)
	const kindLabel = (c: string): string | undefined =>
		(KIND_KEYS as readonly string[]).includes(c) ? t(`kind.${c}`) : undefined
	const typeLabel = (k: string) =>
		(WB_TYPE_KEYS as readonly string[]).includes(k) ? t(`boardType.${k}.label`) : t('boardType.fallback')
	const wbTypes = useMemo(
		() => WB_TYPE_KEYS.map((key) => ({ key, label: t(`boardType.${key}.label`), blurb: t(`boardType.${key}.blurb`) })),
		[t]
	)
	const [room] = useState(resolveRoom)
	const [lanIp, setLanIp] = useState('')
	useEffect(() => {
		// ask the server for its LAN IP so the share URL is reachable from other
		// devices (localhost would point a phone at itself)
		fetch('/api/lan')
			.then((r) => r.json())
			.then((d) => d.ip && setLanIp(d.ip))
			.catch(() => {})
	}, [])
	const isLocalHostPage = ['localhost', '127.0.0.1', '::1', ''].includes(location.hostname)
	const shareHost = isLocalHostPage && lanIp ? lanIp : location.hostname
	const shareUrl = `${location.protocol}//${shareHost}${location.port ? ':' + location.port : ''}${location.pathname}?room=${encodeURIComponent(room)}`

	const { doc, yShapes, yConnectors, yMeta, yFrames, yTranscript, provider, undoMgr, LOCAL } = useMemo(() => {
		const doc = new Y.Doc()
		// ?view=1 唯讀連結:server 在 ws 層丟棄這條連線的文件寫入(UI 隱藏只是 UX,enforce 在 server);
		// key = 房主鑰匙,鎖板時 server 憑它放行房主的寫入
		const params: Record<string, string> = {}
		if (READ_ONLY) params.view = '1'
		let ok = localStorage.getItem(`wb-owner-${room}`)
		if (!ok && !READ_ONLY) {
			// 先發鑰匙再連線:之後 claim 成功(先到先得)這條連線就直接是房主,鎖板不用重連
			ok = genCode(10)
			localStorage.setItem(`wb-owner-${room}`, ok)
		}
		if (ok) params.key = ok
		const provider = new WebsocketProvider(SYNC_WS, room, doc, { params })
		const yShapes = doc.getMap<Sticky>('shapes')
		const yConnectors = doc.getMap<Connector>('connectors')
		const yMeta = doc.getMap<string>('meta') // board type + topic
		const yFrames = doc.getMap<any>('frames') // diagrams on the canvas
		const yTranscript = doc.getArray<any>('transcript') // running word-for-word meeting log
		const LOCAL = { local: true } // origin tag so undo only tracks MY edits, not remote/Mori
		// yFrames is in scope so deleting/moving/renaming a frame is undoable too
		const undoMgr = new Y.UndoManager([yShapes, yConnectors, yFrames], { trackedOrigins: new Set([LOCAL]) })
		;(window as any).__getShapes = () => Array.from(yShapes.values())
		;(window as any).__getConnectors = () => Array.from(yConnectors.values())
		;(window as any).__getFrames = () => Array.from(yFrames.values())
		return { doc, yShapes, yConnectors, yMeta, yFrames, yTranscript, provider, undoMgr, LOCAL }
	}, [room])

	const [shapes, setShapes] = useState<Sticky[]>([])
	const [transcript, setTranscript] = useState<any[]>([]) // running word-for-word meeting log
	const transcriptEndRef = useRef<HTMLDivElement | null>(null)
	const [connectors, setConnectors] = useState<Connector[]>([])
	const [status, setStatus] = useState('connecting')
	// 連線狀態的在地化呈現(y-websocket 給的是英文 raw status)
	const statusLabel = (s: string) =>
		['synced', 'connecting', 'connected', 'disconnected'].includes(s) ? t(`status.${s}`) : s
	// 連線卡超過 5 秒 → 顯示說明 banner(典型場景:免費主機冷啟動 / 斷網)
	const [connSlow, setConnSlow] = useState(false)
	useEffect(() => {
		if (status === 'synced') {
			setConnSlow(false)
			return
		}
		const t = setTimeout(() => setConnSlow(true), 5000)
		return () => clearTimeout(t)
	}, [status])
	const [size, setSize] = useState({ w: window.innerWidth, h: window.innerHeight })
	const [view, setView] = useState({ x: 0, y: 0, scale: 1 }) // canvas pan/zoom
	const [theme, setTheme] = useState(() => (typeof document !== 'undefined' ? document.documentElement.getAttribute('data-theme') || 'light' : 'light'))
	const toggleTheme = () => {
		const next = theme === 'dark' ? 'light' : 'dark'
		setTheme(next)
		document.documentElement.setAttribute('data-theme', next)
		try { localStorage.setItem('mc-theme', next) } catch {}
	}
	const ct = theme === 'dark' ? CANVAS_DARK : CANVAS_LIGHT // canvas (Konva) palette for this theme
	const cc = theme === 'dark' ? CARD_DARK : CARD_LIGHT // 卡面墨色/雜項
	const cardColor = (c: string) => (theme === 'dark' ? COLORS_DARK[c] : COLORS[c]) ?? c
	const kindAccent = (c: string) => (theme === 'dark' ? KIND_ACCENT_DARK[c] : KIND_ACCENT[c]) ?? '#1c1a17'
	// 類型小標只在「會議板」語意下成立(其他板型同一顏色另有意義,標了反而誤導)
	const isMeetingFrame = (frameId?: string) => {
		if (!frameId) return boardTypeKey === 'meeting'
		const f = frames.find((x: any) => x.id === frameId)
		return (f?.type || 'meeting') === 'meeting'
	}
	const [selectedId, setSelectedId] = useState<string | null>(null)
	// each card's editor number (1-based) — same id-sorted order the agent sees, so you can
	// say "把 3 號移到…" as a precise fallback when the AI can't infer which card you mean
	const cardNum = useMemo(() => {
		const m: Record<string, number> = {}
		shapes
			.filter((s: any) => s.type === 'sticky' && !s.note)
			.slice()
			.sort((a: any, b: any) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
			.forEach((s: any, i: number) => (m[s.id] = i + 1))
		return m
	}, [shapes])
	const [selectedConnId, setSelectedConnId] = useState<string | null>(null)
	const [filter, setFilter] = useState<{ type: 'tag' | 'owner'; value: string } | null>(null)
	// Ctrl+F 卡片搜尋:比對 text / owner / tags,Enter 逐筆跳到該卡並短暫高亮
	const [searchOpen, setSearchOpen] = useState(false)
	const [searchQ, setSearchQ] = useState('')
	const [searchIdx, setSearchIdx] = useState(0)
	const searchInputRef = useRef<HTMLInputElement>(null)
	const searchFlashTimer = useRef<any>(null)
	const [connectMode, setConnectMode] = useState(false)
	const [connectFrom, setConnectFrom] = useState<string | null>(null)
	const [editing, setEditing] = useState<{ id: string; value: string } | null>(null)
	const [editingFrame, setEditingFrame] = useState<{ id: string; value: string } | null>(null)
	const [agentText, setAgentText] = useState('') // manual-transcript draft (local only, not synced)
	const [showPaste, setShowPaste] = useState(false) // the paste-transcript option is hidden by default
	const [busy, setBusy] = useState('')
	// 最近一輪 AI 新增的卡 id(誠實範圍:只含「新增」;AI 的修改/刪除 UndoManager 不追遠端,不在此列)
	const [lastAiIds, setLastAiIds] = useState<string[]>([])
	const editRef = useRef<HTMLTextAreaElement>(null)
	const stageRef = useRef<any>(null)
	const dragTs = useRef(0)
	const pinchRef = useRef(0)
	const [shareOpen, setShareOpen] = useState(false)
	const [qrUrl, setQrUrl] = useState('')
	const [joinCode, setJoinCode] = useState('')
	const [roomList, setRoomList] = useState<{ id: string; shapes: number; online: number }[]>([])
	// demo 站 PUBLIC_ROOM_LIST=0 時 /api/rooms 只回 count 不回房號 — 清單空時就顯示這個數字
	const [roomCount, setRoomCount] = useState(0)
	const [panelOpen, setPanelOpen] = useState(window.innerWidth >= 700) // collapse agent panel on small screens
	const [guide, setGuide] = useState(() => !localStorage.getItem('wb-seen-guide')) // first-run onboarding
	// 範例庫:persona 範例板(client/public/examples/*.json),載入即還原成完整示範畫板
	const [examplesOpen, setExamplesOpen] = useState(false)
	const [exampleIndex, setExampleIndex] = useState<any[]>([])
	// 社群範本(client/public/templates/*.json,社群 PR 投稿;索引空就不顯示該區)
	const [templateIndex, setTemplateIndex] = useState<any[]>([])
	// ?board= 深連結:首次同步後只消費一次
	const boardParam = useRef(resolveBoardParam())
	// 互動導覽:spotlight 逐步指向真實按鈕;-1 = 未啟動
	const [tourStep, setTourStep] = useState(-1)
	const [boardTypeKey, setBoardTypeKey] = useState('meeting') // synced board type
	const [boardTopic, setBoardTopic] = useState('')
	const [frames, setFrames] = useState<any[]>([]) // diagrams on the canvas
	const [typePickerOpen, setTypePickerOpen] = useState(false)
	const [newFrameTitle, setNewFrameTitle] = useState('')
	const [exportOpen, setExportOpen] = useState(false)
	const [pngTransparent, setPngTransparent] = useState(false)
	const [exporting, setExporting] = useState(false) // true while capturing the whole board (disables viewport culling)
	const [settingsOpen, setSettingsOpen] = useState(false)
	const [menuOpen, setMenuOpen] = useState(false) // mobile top-bar overflow menu
	const [installPrompt, setInstallPrompt] = useState<any>(null) // deferred PWA install prompt
	const [iosInstallHint, setIosInstallHint] = useState(false)
	const [settings, setSettings] = useState({ localOnly: false, groqKey: true, spacing: 1, autoTidy: true, mode: 'mori', sttSource: 'local', whisperUrl: '', adminLocked: false })
	// bring your own AI: any OpenAI-compatible base + key + model -> visitor's own quota
	const [byo, setByo] = useState(() => ({ base: localStorage.getItem('wb-llm-base') || '', key: localStorage.getItem('wb-llm-key') || '', model: localStorage.getItem('wb-llm-model') || '' }))
	const saveByo = (patch: Partial<typeof byo>) =>
		setByo((b) => {
			const n = { ...b, ...patch }
			localStorage.setItem('wb-llm-base', n.base)
			localStorage.setItem('wb-llm-key', n.key)
			localStorage.setItem('wb-llm-model', n.model)
			return n
		})
	// 設定頁貼的 Groq key(wb-groq-key)也走同一套 BYO header:key 只存這個瀏覽器、
	// 逐次請求帶上,不進 server 全域 —— 訪客之間不共用、也蓋不掉彼此。三格 BYO 都填時以 BYO 為準。
	// X-Lang 一律帶上:server 依它決定 AI 輸出語言(en 時附加英文輸出指令、跳過繁化)。
	const byoHeaders = (): Record<string, string> => {
		const base: Record<string, string> = { 'X-Lang': apiLang() }
		if (byo.base.trim() && byo.key.trim() && byo.model.trim())
			return { ...base, 'X-LLM-Base': byo.base.trim(), 'X-LLM-Key': byo.key.trim(), 'X-LLM-Model': byo.model.trim() }
		const gk = (localStorage.getItem('wb-groq-key') || '').trim()
		if (gk) return { ...base, 'X-LLM-Base': 'https://api.groq.com/openai/v1', 'X-LLM-Key': gk, 'X-LLM-Model': 'openai/gpt-oss-120b' }
		return base
	}
	const [sponsor, setSponsor] = useState<{ url?: string; label?: string; notice?: string }>({})
	const [sponsorHidden, setSponsorHidden] = useState(false)
	const [caps, setCaps] = useState({ moriEar: true, whisperServer: true, groqKey: true })
	// 設定頁貼的 Groq key:存這個瀏覽器(localStorage),AI 請求逐次以 BYO header 帶上(見 byoHeaders)。
	// 僅在主機未鎖時才同步給 server(單機桌面版 loopback 場景:讓雲端 STT 也能用);
	// 公開部署 server 會拒收,key 只在這個瀏覽器生效(供 AI 畫卡,語音辨識仍走站長的 STT 設定)。
	const [groqKeyInput, setGroqKeyInput] = useState(() => localStorage.getItem('wb-groq-key') || '')
	async function saveGroqKey(key: string, adminLocked = settings.adminLocked) {
		localStorage.setItem('wb-groq-key', key)
		if (adminLocked) return
		const r = await fetch(`${SYNC_HTTP}/api/settings`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ groqApiKey: key }) })
			.then((x) => x.json())
			.catch(() => null)
		if (r?.ok) {
			setSettings((s) => ({ ...s, groqKey: r.groqKey }))
			setCaps((c) => ({ ...c, groqKey: r.groqKey, moriEar: r.moriEar ?? c.moriEar, whisperServer: r.whisperServer ?? c.whisperServer }))
		}
	}
	const [cfgInfo, setCfgInfo] = useState({ llmGroqModel: '', llmOllamaModel: '', sttProvider: '', sttGroqModel: '', sttLocalModel: '' })
	const [subtitle, setSubtitle] = useState('') // transient STT caption (UX feedback)
	const subtitleTimer = useRef<any>(null)

	// presence: my identity (persistent name + colour) + everyone else's cursors
	const [myName, setMyName] = useState(() => localStorage.getItem('wb-name') || t('nameGate.guestPrefix') + '-' + genCode(3))
	// prompt for a real name on entry (so people aren't all anonymous '訪客-XXX' / 'Guest-XXX' in a meeting)
	const isGuestName = (n: string) => /^(訪客|Guest)/.test(n)
	const [needName, setNeedName] = useState(() => {
		const n = localStorage.getItem('wb-name')
		return !n || isGuestName(n)
	})
	const [nameDraft, setNameDraft] = useState('')
	const myColor = useMemo(
		() => ['#e11d48', '#0891b2', '#ea580c', '#16a34a', '#9333ea'][Math.floor(Math.random() * 5)],
		[]
	)
	const me = useMemo(() => ({ name: myName, color: myColor }), [myName, myColor])
	// 「先看看就好」的延後補問:第一次錄音/建卡/開分享(名字的價值此刻才成立)再問一次,只問這一次
	const nameReasked = useRef(false)
	function maybeAskName(): boolean {
		if (!nameReasked.current && isGuestName(myName)) {
			nameReasked.current = true
			setNeedName(true)
			return true
		}
		return false
	}
	// append a recognised speech segment to the shared word-for-word meeting log
	const logTranscript = (text: string) => {
		const t = (text || '').trim()
		if (!t) return
		doc.transact(() => yTranscript.push([{ t: new Date().toISOString(), by: me.name, text: t }]))
	}
	useEffect(() => {
		transcriptEndRef.current?.scrollIntoView({ block: 'nearest' })
	}, [transcript.length])
	// keep-alive: while this page is open, ping the server every 5 min so a free host
	// (e.g. Render) doesn't spin the instance down mid-meeting. (Doesn't help the first
	// cold start when nobody is on the page — that needs an external cron.)
	useEffect(() => {
		const id = setInterval(() => {
			fetch(`${SYNC_HTTP}/api/health`, { cache: 'no-store' }).catch(() => {})
		}, 5 * 60 * 1000)
		return () => clearInterval(id)
	}, [])
	// capture the PWA install prompt (Chrome/Edge/Android) so we can offer an "安裝 App" button
	useEffect(() => {
		const onBIP = (e: any) => {
			e.preventDefault()
			setInstallPrompt(e)
		}
		window.addEventListener('beforeinstallprompt', onBIP)
		window.addEventListener('appinstalled', () => setInstallPrompt(null))
		return () => window.removeEventListener('beforeinstallprompt', onBIP)
	}, [])
	useEffect(() => {
		localStorage.setItem('wb-name', myName)
		;(provider as any).awareness.setLocalStateField('user', me)
	}, [me, provider, myName])
	const [cursors, setCursors] = useState<{ id: number; name: string; color: string; x: number; y: number }[]>([])
	const cursorTs = useRef(0)

	// --- yjs mutations (tagged LOCAL so the UndoManager tracks them) ---
	const tx = (fn: () => void) => doc.transact(fn, LOCAL)
	const patchShape = (id: string, patch: Partial<Sticky>) => {
		const cur = yShapes.get(id)
		if (cur) tx(() => yShapes.set(id, { ...cur, ...patch }))
	}
	const patchFrame = (id: string, patch: any) => {
		const cur = yFrames.get(id)
		if (cur) tx(() => yFrames.set(id, { ...cur, ...patch }))
	}
	// move a frame and all its cards together by (dx,dy)
	const moveFrame = (f: any, dx: number, dy: number) => {
		tx(() => {
			yFrames.set(f.id, { ...f, x: f.x + dx, y: f.y + dy })
			for (const s of yShapes.values()) if ((s as any).frameId === f.id) yShapes.set(s.id, { ...s, x: s.x + dx, y: s.y + dy } as any)
		})
	}
	const addSticky = (x: number, y: number, text = '', color = 'yellow') => {
		if (READ_ONLY) return ''
		maybeAskName() // 不擋建卡,只順勢補問一次
		const id = `sticky-${Math.random().toString(36).slice(2, 10)}`
		tx(() => yShapes.set(id, { id, x, y, w: 200, h: 200, text, color, drawnBy: 'user' }))
		return id
	}
	// 備註 — a sticky-style annotation (note:true) you can drop on ANY diagram. It's NOT a
	// diagram node: the server's auto-arrange + AI both ignore note cards, so they stay put.
	const addNote = (x: number, y: number, text = '') => {
		const id = `note-${Math.random().toString(36).slice(2, 10)}`
		tx(() => yShapes.set(id, { id, x, y, w: 200, h: 200, text, color: 'note', note: true, drawnBy: 'user' } as any))
		return id
	}
	const deleteSticky = (id: string) =>
		tx(() => {
			yShapes.delete(id)
			for (const [cid, c] of yConnectors) if (c.from === id || c.to === id) yConnectors.delete(cid)
		})
	// delete a whole diagram (frame) + the cards inside it + their connectors
	const deleteFrame = (fid: string) => {
		const inside = ([...yShapes.values()] as any[]).filter((s) => s.frameId === fid)
		const title = frames.find((f) => f.id === fid)?.title || ''
		if (inside.length && !window.confirm(t('confirm.deleteFrame', { title, count: inside.length }))) return
		const ids = new Set(inside.map((s) => s.id))
		const connIds = ([...yConnectors] as any[]).filter(([, c]) => ids.has(c.from) || ids.has(c.to)).map(([cid]) => cid)
		tx(() => {
			yFrames.delete(fid)
			for (const id of ids) yShapes.delete(id)
			for (const cid of connIds) yConnectors.delete(cid)
		})
	}
	const addConnector = (from: string, to: string) => {
		const id = `conn-${Math.random().toString(36).slice(2, 10)}`
		tx(() => yConnectors.set(id, { id, from, to }))
	}
	const deleteConnector = (id: string) => tx(() => yConnectors.delete(id))
	const clearAll = () =>
		tx(() => {
			yShapes.clear()
			yConnectors.clear()
			yFrames.clear() // 圖框也要清,否則「清空」後空圖框留在板上
			if (yTranscript.length) yTranscript.delete(0, yTranscript.length) // 連逐字記錄一起清
		})

	function exportMd() {
		// lang 目前只給未來擴充用(.md 匯出是確定性轉換,區段標題仍為 zh-TW)
		window.open(`${SYNC_HTTP}/api/export/${encodeURIComponent(room)}?lang=${apiLang()}`, '_blank')
	}
	// a self-contained, styled HTML board record: type-aware AI summary + optional
	// transcript. Double-click the .html to read it in any browser (no tools needed).
	async function exportHtml() {
		setBusy(t('busy.htmlExporting'))
		const esc = (s: string) => (s || '').replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' } as any)[c])
		const bold = (s: string) => s.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
		const mdToHtml = (md: string) => {
			let out = '',
				inList = false
			const close = () => {
				if (inList) {
					out += '</ul>'
					inList = false
				}
			}
			for (const raw of (md || '').split('\n')) {
				const line = raw.trimEnd()
				const li = (t: string) => `<li>${bold(esc(t))}</li>`
				if (/^###\s/.test(line)) (close(), (out += `<h3>${bold(esc(line.slice(4)))}</h3>`))
				else if (/^##\s/.test(line)) (close(), (out += `<h2>${bold(esc(line.slice(3)))}</h2>`))
				else if (/^#\s/.test(line)) (close(), (out += `<h1>${bold(esc(line.slice(2)))}</h1>`))
				else if (/^[-*]\s/.test(line)) {
					if (!inList) {
						out += '<ul>'
						inList = true
					}
					out += li(line.slice(2))
				} else if (line.trim() === '') close()
				else (close(), (out += `<p>${bold(esc(line))}</p>`))
			}
			close()
			return out
		}
		let summaryMd = ''
		try {
			summaryMd = await fetch(`${SYNC_HTTP}/api/summary/${encodeURIComponent(room)}`, { headers: byoHeaders() }).then((x) => x.text())
		} catch {
			summaryMd = t('htmlDoc.summaryFailed')
		}
		// embed a snapshot of the whole board (self-contained dataURL, capped at 1200px)
		let boardImg = ''
		try {
			const url = await boardPng(false, 1200)
			if (url) boardImg = `<section><img class="board" src="${url}" alt="${t('htmlDoc.boardAlt')}"></section>`
		} catch {}
		const tHtml = transcript.length
			? transcript.map((e: any) => `<div class="t"><span class="m">${esc((e.t || '').slice(11, 16))} ${esc(e.by || '')}</span>${esc(e.text || '')}</div>`).join('')
			: `<p class="muted">${t('htmlDoc.noTranscript')}</p>`
		const date = new Date().toLocaleString(uiLang)
		const html = `<!doctype html><html lang="${uiLang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${t('htmlDoc.title')} · ${esc(room)}</title>
<style>
:root{--ink:#1c1a17;--soft:#6b655c;--line:#e7e1d6;--accent:#b4530a;--bg:#faf7f1}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font-family:'Hanken Grotesk','Noto Sans TC','PingFang TC','Microsoft JhengHei',system-ui,sans-serif;line-height:1.6}
.wrap{max-width:760px;margin:0 auto;padding:40px 24px 64px}
header{border-bottom:2px solid var(--accent);padding-bottom:14px;margin-bottom:24px}
h1{font-size:26px;margin:0 0 4px}.sub{color:var(--soft);font-size:13px}
h2{font-size:19px;margin:28px 0 8px;color:var(--accent)}h3{font-size:15px;margin:16px 0 6px}
ul{margin:6px 0 12px;padding-left:22px}li{margin:3px 0}p{margin:8px 0}
.transcript{margin-top:8px}.t{font-size:13px;padding:5px 0;border-bottom:1px solid var(--line)}
.t .m{color:var(--soft);font-size:11px;margin-right:8px;white-space:nowrap}
.muted{color:var(--soft)}footer{margin-top:40px;color:var(--soft);font-size:12px;text-align:center}
.board{display:block;width:100%;height:auto;border:1px solid var(--line);border-radius:12px;margin:18px 0 6px}
@media print{body{background:#fff}.wrap{max-width:none}}
</style></head><body><div class="wrap">
<header><h1>${t('htmlDoc.title')}</h1><div class="sub">${t('htmlDoc.room')} ${esc(room)}${boardTopic ? ' · ' + esc(boardTopic) : ''} · ${esc(date)}</div></header>
${boardImg}
<section>${mdToHtml(summaryMd)}</section>
<section class="transcript"><h2>${t('htmlDoc.transcript')}</h2>${tHtml}</section>
<footer>${t('htmlDoc.footer')}</footer>
</div></body></html>`
		const blob = new Blob([html], { type: 'text/html;charset=utf-8' })
		const a = document.createElement('a')
		a.href = URL.createObjectURL(blob)
		a.download = `${t('htmlDoc.fileName')}-${room}-${new Date().toISOString().slice(0, 10)}.html`
		a.click()
		setTimeout(() => URL.revokeObjectURL(a.href), 1000)
		setBusy(t('busy.htmlExported'))
	}
	function joinRoom() {
		const c = joinCode.trim().toUpperCase()
		if (c && c !== room) location.href = `${location.pathname}?room=${encodeURIComponent(c)}`
	}
	function tidy() {
		setBusy(t('busy.tidying'))
		fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/tidy`, { method: 'POST' })
			.then(() => setBusy(t('busy.tidied')))
			.catch(() => setBusy(t('busy.tidyFailed')))
	}
	async function saveSettings(patch: Partial<typeof settings>) {
		const res = await fetch(`${SYNC_HTTP}/api/settings`, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json', ...byoHeaders() },
			body: JSON.stringify(patch),
		}).catch(() => null)
		// 設了 ADMIN_TOKEN 的部署:沒帶正確 X-Admin-Token 一律 401,主機設定改不動
		if (res?.status === 401) {
			setSettings((s) => ({ ...s, adminLocked: true }))
			setBusy(t('settings.hostLocked'))
			return
		}
		const r = await res?.json().catch(() => null)
		if (r?.ok) {
			setSettings({ localOnly: r.localOnly, groqKey: r.groqKey, spacing: r.spacing, autoTidy: r.autoTidy, mode: r.mode, sttSource: r.sttSource, whisperUrl: r.whisperUrl || '', adminLocked: !!r.adminLocked })
			setCaps({ moriEar: r.moriEar, whisperServer: r.whisperServer, groqKey: r.groqKey })
		} else if (r?.error) {
			// 例:非 loopback 訪客試圖改主機級欄位(whisperUrl / 模式等)被擋
			setBusy(r.error)
		}
		if (patch.spacing !== undefined) tidy() // re-arrange so the new spacing shows immediately
	}
	// set this board's type/topic (server-side, authoritative) then re-arrange
	async function setBoardType(key: string, topic?: string) {
		await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/meta`, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json', ...byoHeaders() },
			body: JSON.stringify({ type: key, ...(topic !== undefined ? { topic } : {}) }),
		}).catch(() => {})
		tidy()
	}
	// transient STT caption — UX hint so the speaker sees what was heard, fades in 3s
	function showSubtitle(text: string) {
		if (!text) return
		setSubtitle(text)
		if (subtitleTimer.current) clearTimeout(subtitleTimer.current)
		subtitleTimer.current = setTimeout(() => setSubtitle(''), 3000)
	}
	function downloadUri(uri: string) {
		const a = document.createElement('a')
		a.href = uri
		a.download = `whiteboard-${room}.png`
		a.click()
	}
	// world-space bounding box of everything on the board (frames + cards)
	function contentBBox(): { x: number; y: number; w: number; h: number } | null {
		const rects = [
			...frames.map((f: any) => ({ x: f.x, y: f.y, w: f.w, h: f.h })),
			...shapes.map((s) => ({ x: s.x, y: s.y, w: s.w, h: s.h })),
		]
		if (!rects.length) return null
		let minX = Infinity
		let minY = Infinity
		let maxX = -Infinity
		let maxY = -Infinity
		for (const r of rects) {
			minX = Math.min(minX, r.x)
			minY = Math.min(minY, r.y)
			maxX = Math.max(maxX, r.x + r.w)
			maxY = Math.max(maxY, r.y + r.h)
		}
		return { x: minX, y: minY, w: maxX - minX, h: maxY - minY }
	}
	// render the WHOLE board (content bbox + padding, not just the viewport) to a PNG
	// dataURL. The stage transform is temporarily reset so toDataURL crops in world
	// coords, then restored — the on-screen view never visibly moves.
	// maxPx caps the output's larger edge (pixelRatio shrinks for huge boards).
	async function boardPng(transparent: boolean, maxPx = 4096): Promise<string | null> {
		const stage = stageRef.current
		const bbox = contentBBox()
		if (!stage || !bbox) return null
		const PAD = 30
		const w = bbox.w + PAD * 2
		const h = bbox.h + PAD * 2
		const pixelRatio = Math.min(2, maxPx / w, maxPx / h)
		const prev = { x: stage.x(), y: stage.y(), scaleX: stage.scaleX(), scaleY: stage.scaleY() }
		flushSync(() => setExporting(true)) // mount viewport-culled (off-screen) nodes before capture
		stage.position({ x: PAD - bbox.x, y: PAD - bbox.y })
		stage.scale({ x: 1, y: 1 })
		const dataUrl = stage.toDataURL({ x: 0, y: 0, width: w, height: h, pixelRatio })
		stage.position({ x: prev.x, y: prev.y })
		stage.scale({ x: prev.scaleX, y: prev.scaleY })
		flushSync(() => setExporting(false))
		if (transparent) return dataUrl
		// composite onto paper matching the CURRENT theme (Konva canvas itself is transparent)
		const paper = getComputedStyle(document.documentElement).getPropertyValue('--paper').trim() || '#f1ece1'
		return new Promise((resolve) => {
			const img = new Image()
			img.onload = () => {
				const c = document.createElement('canvas')
				c.width = img.width
				c.height = img.height
				const ctx = c.getContext('2d')!
				ctx.fillStyle = paper
				ctx.fillRect(0, 0, c.width, c.height)
				ctx.drawImage(img, 0, 0)
				resolve(c.toDataURL('image/png'))
			}
			img.onerror = () => resolve(dataUrl)
			img.src = dataUrl
		})
	}
	// copy the whole-board PNG to the clipboard (paste straight into chats / docs)
	async function copyPngToClipboard() {
		if (!navigator.clipboard?.write || typeof ClipboardItem === 'undefined') {
			setBusy(t('busy.pngCopyUnsupported'))
			return
		}
		setBusy(t('busy.pngCopying'))
		try {
			// pass a Promise<Blob> so Safari accepts the write inside the user gesture
			const blobPromise = (async () => {
				const url = await boardPng(pngTransparent)
				if (!url) throw new Error('empty board')
				return await (await fetch(url)).blob()
			})()
			await navigator.clipboard.write([new ClipboardItem({ 'image/png': blobPromise })])
			setBusy(t('busy.pngCopied'))
		} catch {
			setBusy(contentBBox() ? t('busy.pngCopyFailed') : t('busy.emptyBoardCopy'))
		}
	}
	// the Konva stage canvas is transparent (the paper grid is CSS, not drawn).
	// transparent=true exports as-is; otherwise composite onto a paper background.
	function exportBoard() {
			const board = {
				format: 'mori-canvas/v1',
				exportedAt: new Date().toISOString(),
				room,
				meta: { type: yMeta.get('type') || 'meeting', topic: yMeta.get('topic') || '' },
				frames: Array.from(yFrames.values()),
				shapes: Array.from(yShapes.values()),
				connectors: Array.from(yConnectors.values()),
				transcript: yTranscript.toArray(),
			}
			const blob = new Blob([JSON.stringify(board, null, 2)], { type: 'application/json' })
			const a = document.createElement('a')
			a.href = URL.createObjectURL(blob)
			a.download = `mori-canvas-${room}-${new Date().toISOString().slice(0, 10)}.json`
			a.click()
			setTimeout(() => URL.revokeObjectURL(a.href), 1000)
		}
		// 把一份 mori-canvas/v1 畫板資料整批寫進共享 doc(匯入還原與載入範例共用)
		function applyBoardData(data: any) {
			tx(() => {
				for (const k of [...yShapes.keys()]) yShapes.delete(k)
				for (const k of [...yConnectors.keys()]) yConnectors.delete(k)
				for (const k of [...yFrames.keys()]) yFrames.delete(k)
				if (yTranscript.length) yTranscript.delete(0, yTranscript.length)
				for (const f of data.frames || []) if (f?.id) yFrames.set(f.id, f)
				for (const s of data.shapes || []) if (s?.id) yShapes.set(s.id, s)
				for (const c of data.connectors || []) if (c?.id) yConnectors.set(c.id, c)
				if (Array.isArray(data.transcript) && data.transcript.length) yTranscript.push(data.transcript)
				if (data.meta?.type) yMeta.set('type', data.meta.type)
				if (data.meta?.topic != null) yMeta.set('topic', String(data.meta.topic))
			})
		}
		async function importBoard(file: File) {
			let data: any
			try {
				data = JSON.parse(await file.text())
			} catch {
				setBusy(t('busy.importBadJson'))
				return
			}
			if (!data || !Array.isArray(data.shapes)) {
				setBusy(t('busy.importBadFormat'))
				return
			}
			if (yShapes.size > 0 && !window.confirm(t('confirm.importOverwrite'))) return
			applyBoardData(data)
			setBusy(t('busy.imported', { cards: data.shapes.length, frames: (data.frames || []).length }))
			setExportOpen(false)
		}
		// 範例庫:索引懶載入(內建 examples + 社群 templates)+ 載入單一範例(套資料後叫 server 依板型排版)
		async function openExamples() {
			setExamplesOpen(true)
			if (!exampleIndex.length) {
				const d = await fetch('examples/index.json').then((r) => r.json()).catch(() => null)
				if (d?.examples) setExampleIndex(d.examples)
			}
			if (!templateIndex.length) {
				const d = await fetch('templates/index.json').then((r) => r.json()).catch(() => null)
				if (Array.isArray(d?.examples) && d.examples.length) setTemplateIndex(d.examples)
			}
		}
		async function loadExample(id: string, dir: 'examples' | 'templates' = 'examples') {
			const data = await fetch(`${dir}/${id}.json`).then((r) => r.json()).catch(() => null)
			if (!data || !Array.isArray(data.shapes)) {
				setBusy(t('examples.loadFailed'))
				return
			}
			if (yShapes.size > 0 && !window.confirm(t('examples.confirmOverwrite'))) return
			applyBoardData(data)
			await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/tidy`, { method: 'POST' }).catch(() => null)
			setExamplesOpen(false)
			setGuide(false)
			setBusy(t('examples.loaded', { cards: data.shapes.length, frames: (data.frames || []).length }))
		}
		// 範例分享連結:新隨機房號 + board id —— 別人點開,新房自動長出這份範本
		function copyBoardLink(id: string) {
			const url = `${location.origin}${location.pathname}?room=${genCode()}&board=${id}`
			navigator.clipboard
				?.writeText(url)
				.then(() => setBusy(t('examples.linkCopied')))
				.catch(() => window.prompt(t('examples.copyPrompt'), url))
		}
		function pickAndImportBoard() {
			const inp = document.createElement('input')
			inp.type = 'file'
			inp.accept = 'application/json,.json'
			inp.onchange = () => {
				const f = inp.files?.[0]
				if (f) importBoard(f)
			}
			inp.click()
		}
		async function exportPng(transparent: boolean) {
		setBusy(t('busy.pngExporting'))
		const url = await boardPng(transparent)
		if (!url) {
			setBusy(t('busy.emptyBoardExport'))
			return
		}
		downloadUri(url)
		setBusy(t('busy.pngExported'))
	}

	useEffect(() => {
		const sync = () => setShapes(Array.from(yShapes.values()))
		const syncC = () => setConnectors(Array.from(yConnectors.values()))
		const syncMeta = () => {
			setBoardTypeKey((yMeta.get('type') as string) || 'meeting')
			setBoardTopic((yMeta.get('topic') as string) || '')
		}
		const syncFrames = () => setFrames(Array.from(yFrames.values()))
		const syncT = () => setTranscript(Array.from(yTranscript.toArray()))
		sync()
		syncC()
		syncMeta()
		syncFrames()
		syncT()
		yShapes.observe(sync)
		yConnectors.observe(syncC)
		yMeta.observe(syncMeta)
		yFrames.observe(syncFrames)
		yTranscript.observe(syncT)
		// presence: track everyone else's cursors (Mori + other humans)
		const aw = (provider as any).awareness
		const updateCursors = () => {
			const out: { id: number; name: string; color: string; x: number; y: number }[] = []
			aw.getStates().forEach((st: any, cid: number) => {
				if (cid === aw.clientID || !st?.cursor || !st?.user) return
				out.push({ id: cid, name: st.user.name, color: st.user.color, x: st.cursor.x, y: st.cursor.y })
			})
			setCursors(out)
		}
		aw.on('change', updateCursors)
		aw.setLocalStateField('user', me)
		updateCursors()
		const onStatus = (e: { status: string }) => setStatus(e.status)
		provider.on('status', onStatus)
		provider.on('sync', (s: boolean) => s && setStatus('synced'))
		const onResize = () => setSize({ w: window.innerWidth, h: window.innerHeight })
		window.addEventListener('resize', onResize)
		return () => {
			yShapes.unobserve(sync)
			yConnectors.unobserve(syncC)
			yMeta.unobserve(syncMeta)
			yFrames.unobserve(syncFrames)
			aw.off('change', updateCursors)
			provider.off('status', onStatus)
			window.removeEventListener('resize', onResize)
			provider.destroy()
		}
	}, [yShapes, yConnectors, yMeta, yFrames, provider])

	// ?board= 深連結:首次同步完成後消費一次 —— 房間是空的才套範本(examples/ 優先、templates/ 後備),
	// 已有內容就略過,避免蓋掉別人正在用的板。
	const boardAutoloadDone = useRef(false)
	useEffect(() => {
		const id = boardParam.current
		if (!id || boardAutoloadDone.current || status !== 'synced') return
		boardAutoloadDone.current = true
		if (yShapes.size > 0 || yFrames.size > 0) {
			setBusy(t('board.alreadyHasContent'))
			return
		}
		;(async () => {
			const get = (dir: string) =>
				fetch(`${dir}/${id}.json`).then((r) => (r.ok ? r.json() : null)).catch(() => null)
			const data = (await get('examples')) || (await get('templates'))
			if (!data || !Array.isArray(data.shapes)) {
				setBusy(t('board.templateNotFound', { id }))
				return
			}
			applyBoardData(data)
			await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/tidy`, { method: 'POST' }).catch(() => null)
			setGuide(false)
			setBusy(t('board.templateLoaded', { cards: data.shapes.length, frames: (data.frames || []).length }))
		})()
	}, [status])

	// keyboard: undo/redo + delete (but not while editing text)
	useEffect(() => {
		const onKey = (e: KeyboardEvent) => {
			if (editing) return // 輸入卡片文字時不攔任何快捷鍵
			// 唯讀檢視:刪除與 undo/redo 不動板(搜尋/Esc 照常);寫入封鎖本體在 server
			if (READ_ONLY && (e.key === 'Delete' || e.key === 'Backspace' || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z'))) return
			const mod = e.ctrlKey || e.metaKey
			if (mod && e.key.toLowerCase() === 'f') {
				// 攔下瀏覽器內建搜尋,開卡片搜尋列(已開著就重新聚焦)
				e.preventDefault()
				setSearchOpen(true)
				searchInputRef.current?.select()
				return
			}
			// 焦點在輸入框(搜尋列、改名、負責人…)時,其餘板面快捷鍵全部不攔
			const el = e.target as HTMLElement | null
			if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable)) {
				if (e.key === 'Escape' && searchOpen) closeSearch()
				return
			}
			if (mod && e.key.toLowerCase() === 'z') {
				e.preventDefault()
				if (e.shiftKey) undoMgr.redo()
				else undoMgr.undo()
				return
			}
			if (mod && e.key.toLowerCase() === 'y') {
				e.preventDefault()
				undoMgr.redo()
				return
			}
			if (e.key === 'Delete' || e.key === 'Backspace') {
				if (selectedId) {
					e.preventDefault()
					deleteSticky(selectedId)
					setSelectedId(null)
				} else if (selectedConnId) {
					e.preventDefault()
					deleteConnector(selectedConnId)
					setSelectedConnId(null)
				}
			}
			if (e.key === 'Escape') {
				if (searchOpen) closeSearch()
				setSelectedId(null)
				setSelectedConnId(null)
				setConnectFrom(null)
			}
		}
		window.addEventListener('keydown', onKey)
		return () => window.removeEventListener('keydown', onKey)
	}, [selectedId, selectedConnId, editing, undoMgr, searchOpen])

	useEffect(() => {
		if (editing) editRef.current?.focus()
	}, [editing])

	// re-render once the web fonts load so Konva re-measures canvas text crisply
	const [, setFontReady] = useState(false)
	useEffect(() => {
		;(document as any).fonts?.ready.then(() => setFontReady(true))
	}, [])
	useEffect(() => {
		fetch(`${SYNC_HTTP}/api/settings`)
			.then((x) => x.json())
			.then((r) => {
				if (!r?.ok) return
				setSettings({ localOnly: r.localOnly, groqKey: r.groqKey, spacing: r.spacing, autoTidy: r.autoTidy, mode: r.mode, sttSource: r.sttSource, whisperUrl: r.whisperUrl || '', adminLocked: !!r.adminLocked })
				setCaps({ moriEar: r.moriEar, whisperServer: r.whisperServer, groqKey: r.groqKey })
				setCfgInfo({ llmGroqModel: r.llmGroqModel, llmOllamaModel: r.llmOllamaModel, sttProvider: r.sttProvider, sttGroqModel: r.sttGroqModel, sttLocalModel: r.sttLocalModel })
				setSponsor({ url: r.sponsorUrl || '', label: r.sponsorLabel || '', notice: r.demoNotice || '' })
				// server's runtime Groq key is lost on restart — if this browser stashed one and
				// the server now reports none, push it back so cloud STT stays unlocked.
				// 只在主機未鎖時補送(單機 loopback 場景);鎖定部署的 key 走 BYO header 就好
				const stored = localStorage.getItem('wb-groq-key')
				if (stored && !r.groqKey && !r.adminLocked) saveGroqKey(stored, !!r.adminLocked)
			})
			.catch(() => {})
	}, [])

	useEffect(() => {
		if (!shareOpen) return
		QRCode.toDataURL(shareUrl, { width: 240, margin: 1 }).then(setQrUrl).catch(() => setQrUrl(''))
		fetch('/api/rooms')
			.then((r) => r.json())
			.then((d) => {
				setRoomList(d.rooms || [])
				setRoomCount(d.count ?? (d.rooms || []).length)
			})
			.catch(() => {})
	}, [shareOpen, shareUrl])

	async function endThisRoom() {
		if (!window.confirm(t('confirm.endRoom', { room }))) return
		await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/end`, { method: 'POST' }).catch(() => {})
	}

	// 房主與鎖板:第一個進房的人自動認領鑰匙(server 先到先得);鎖板後其他人的寫入在 ws 層被丟棄
	const [roomLocked, setRoomLocked] = useState(false)
	useEffect(() => {
		if (status !== 'synced') return
		;(async () => {
			const m = await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/meta`).then((r) => r.json()).catch(() => null)
			if (!m?.ok) return
			setRoomLocked(!!m.locked)
			const key = localStorage.getItem(`wb-owner-${room}`)
			if (!m.hasOwner && key && !READ_ONLY) {
				await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/claim`, {
					method: 'POST',
					headers: { 'Content-Type': 'application/json' },
					body: JSON.stringify({ key }),
				}).catch(() => null)
			}
		})()
	}, [status])
	async function toggleLock() {
		const key = localStorage.getItem(`wb-owner-${room}`) || ''
		const r = await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/lock`, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: JSON.stringify({ key, locked: !roomLocked }),
		}).then((x) => x.json()).catch(() => null)
		if (r?.ok) {
			setRoomLocked(!!r.locked)
			setBusy(r.locked ? t('share.locked') : t('share.unlocked'))
		} else setBusy(r?.error || t('share.lockFailed'))
	}
	function copyViewLink() {
		const url = `${shareUrl}&view=1`
		navigator.clipboard
			?.writeText(url)
			.then(() => setBusy(t('share.viewLinkCopied')))
			.catch(() => window.prompt(t('share.viewLinkPrompt'), url))
	}

	const byId = (id: string) => shapes.find((s) => s.id === id)
	const matchesFilter = (s: Sticky) =>
		!filter ||
		(filter.type === 'tag' ? (s.tags || []).includes(filter.value) : s.owner === filter.value || s.drawnBy === filter.value)
	// 搜尋命中清單(不分大小寫,比對 text / owner / tags)
	const searchHits = useMemo(() => {
		const q = searchQ.trim().toLowerCase()
		if (!q) return [] as Sticky[]
		return shapes.filter(
			(s) =>
				(s.text || '').toLowerCase().includes(q) ||
				(s.owner || '').toLowerCase().includes(q) ||
				(s.tags || []).some((t) => t.toLowerCase().includes(q))
		)
	}, [shapes, searchQ])
	// 跳到第 i 筆命中:平移視圖(維持目前縮放)讓該卡置中,並借用 selected 樣式短暫高亮
	function focusHit(i: number) {
		const n = searchHits.length
		if (!n) return
		const idx = ((i % n) + n) % n
		setSearchIdx(idx)
		const s = searchHits[idx]
		const scale = view.scale || 1
		setView({ scale, x: size.w / 2 - (s.x + s.w / 2) * scale, y: size.h / 2 - (s.y + s.h / 2) * scale })
		setSelectedId(s.id)
		clearTimeout(searchFlashTimer.current)
		searchFlashTimer.current = setTimeout(() => setSelectedId((cur) => (cur === s.id ? null : cur)), 1600)
	}
	function closeSearch() {
		setSearchOpen(false)
		setSearchQ('')
		setSearchIdx(0)
	}
	// 邊打邊跳:查詢字串一變就跳到第一筆命中(只跟 searchQ,不跟遠端 shapes 變動)
	useEffect(() => {
		if (!searchOpen || !searchQ.trim()) return
		if (searchHits.length) focusHit(0)
		else setSearchIdx(0)
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [searchQ])
	// --- viewport culling: skip rendering stickies/connectors far outside the view.
	// Margin 200 screen px so fast pans never show pop-in; the dragged / selected /
	// editing / pending-connect card is always rendered (a culled node mid-drag would
	// unmount under the pointer), and exports render everything.
	const CULL_MARGIN = 200
	const viewWorld = {
		x: (-view.x - CULL_MARGIN) / view.scale,
		y: (-view.y - CULL_MARGIN) / view.scale,
		w: (size.w + CULL_MARGIN * 2) / view.scale,
		h: (size.h + CULL_MARGIN * 2) / view.scale,
	}
	const rectInView = (x: number, y: number, w: number, h: number) =>
		x + w > viewWorld.x && x < viewWorld.x + viewWorld.w && y + h > viewWorld.y && y < viewWorld.y + viewWorld.h
	const stickyVisible = (s: Sticky) =>
		exporting || s.id === selectedId || s.id === editing?.id || s.id === connectFrom || rectInView(s.x, s.y, s.w, s.h)
	// a connector (straight or elbow) always stays inside the union bbox of its two cards
	const connVisible = (c: Connector, a: Sticky, b: Sticky) => {
		if (exporting || c.id === selectedConnId) return true
		const x = Math.min(a.x, b.x)
		const y = Math.min(a.y, b.y)
		return rectInView(x, y, Math.max(a.x + a.w, b.x + b.w) - x, Math.max(a.y + a.h, b.y + b.h) - y)
	}
	// which frame (diagram) contains a canvas point — topmost wins
	const frameAt = (cx: number, cy: number) => {
		for (let i = frames.length - 1; i >= 0; i--) {
			const f = frames[i]
			if (cx >= f.x && cx <= f.x + f.w && cy >= f.y && cy <= f.y + f.h) return f
		}
		return null
	}

	function onStickyClick(s: Sticky) {
		if (connectMode) {
			if (!connectFrom) setConnectFrom(s.id)
			else if (connectFrom !== s.id) {
				addConnector(connectFrom, s.id)
				setConnectFrom(null)
			}
			return
		}
		setSelectedId(s.id)
	}

	function onStageDblClick(e: any) {
		if (READ_ONLY) return
		// only when clicking empty canvas (target is the stage itself)
		if (e.target !== e.target.getStage()) return
		// relative pointer position accounts for pan/zoom -> canvas coords
		const pos = e.target.getStage().getRelativePointerPosition()
		const id = addSticky(pos.x - 100, pos.y - 100, '', 'yellow')
		setEditing({ id, value: '' })
		setSelectedId(id)
	}

	function onWheel(e: any) {
		e.evt.preventDefault()
		const stage = e.target.getStage()
		const pointer = stage.getPointerPosition()
		const old = view.scale
		const worldX = (pointer.x - view.x) / old
		const worldY = (pointer.y - view.y) / old
		const next = Math.max(0.25, Math.min(3, e.evt.deltaY > 0 ? old / 1.1 : old * 1.1))
		setView({ scale: next, x: pointer.x - worldX * next, y: pointer.y - worldY * next })
	}

	function onTouchMove(e: any) {
		const t = e.evt.touches
		if (!t || t.length !== 2) return
		e.evt.preventDefault()
		const stage = e.target.getStage()
		stage.stopDrag() // two fingers = zoom, not pan
		const p1 = { x: t[0].clientX, y: t[0].clientY }
		const p2 = { x: t[1].clientX, y: t[1].clientY }
		const dist = Math.hypot(p2.x - p1.x, p2.y - p1.y)
		const cx = (p1.x + p2.x) / 2
		const cy = (p1.y + p2.y) / 2
		if (pinchRef.current) {
			const old = view.scale
			const wx = (cx - view.x) / old
			const wy = (cy - view.y) / old
			const next = Math.max(0.25, Math.min(3, old * (dist / pinchRef.current)))
			setView({ scale: next, x: cx - wx * next, y: cy - wy * next })
		}
		pinchRef.current = dist
	}
	function onTouchEnd() {
		pinchRef.current = 0
	}

	function publishCursor(e: any) {
		const now = Date.now()
		if (now - cursorTs.current < 50) return
		cursorTs.current = now
		const p = e.target.getStage().getRelativePointerPosition()
		if (p) (provider as any).awareness.setLocalState({ user: me, cursor: { x: p.x, y: p.y } })
	}
	function clearCursor() {
		;(provider as any).awareness.setLocalState({ user: me, cursor: null })
	}

	// react to an agent response: a recognised voice command (apply view + show
	// label) or normal content (show how many cards it made)
	function applyAgentResponse(r: any, prefix = '') {
		if (!r || !r.ok) {
			setBusy(r?.error ? t('agent.error', { message: r.error }) : t('agent.errorPlain'))
			return
		}
		if (r.intent === 'command') {
			const c = r.command
			if (c?.action === 'filter') setFilter({ type: c.by === 'tag' ? 'tag' : 'owner', value: c.value })
			else if (c?.action === 'clearFilter') setFilter(null)
			setBusy(t('agent.command', { label: r.commandLabel || t('agent.commandDone') }))
		} else {
			const fl = r.frameLabel ? `${r.frameLabel} · ` : ''
			setBusy(t('agent.added', { prefix: `${prefix}${fl}`, cards: r.added?.length ?? r.stickies ?? 0, connectors: r.connectors ?? 0 }))
			// 每一輪 content 回應都更新「上一輪 AI 新增」清單(被跳過的段落沒有 ids,不動清單)
			if (Array.isArray(r.ids)) setLastAiIds(r.ids)
		}
	}
	// 撤銷上一輪 AI:移除那一輪新增的卡與其相關連線(AI 的修改/刪除不在此列)
	function undoLastAi() {
		const ids = new Set(lastAiIds.filter((id) => yShapes.has(id)))
		if (ids.size) {
			tx(() => {
				for (const [cid, c] of yConnectors) if (ids.has(c.from) || ids.has(c.to)) yConnectors.delete(cid)
				for (const id of ids) yShapes.delete(id)
			})
		}
		setLastAiIds([])
		setBusy(ids.size ? t('agent.undoRemoved', { count: ids.size }) : t('agent.undoGone'))
	}
	async function addFrame(type: string, title: string) {
		await fetch(`${SYNC_HTTP}/api/rooms/${encodeURIComponent(room)}/frames`, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json', ...byoHeaders() },
			body: JSON.stringify({ type, title }),
		}).catch(() => {})
	}

	async function runAgent() {
		if (!agentText.trim()) return
		setBusy(t('agent.thinking'))
		try {
			const r = await fetch(`${SYNC_HTTP}/api/agent/${encodeURIComponent(room)}`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json', ...byoHeaders() },
				body: JSON.stringify({ transcript: agentText, by: me.name }),
			}).then((x) => x.json())
			applyAgentResponse(r)
		} catch (e) {
			setBusy(t('agent.error', { message: (e as Error).message }))
		}
	}

	// voice: mic -> /api/voice -> ear -> agent -> board
	const recRef = useRef<MediaRecorder | null>(null)
	const [recording, setRecording] = useState(false)

	// dictate a single card's text by voice (from the per-card popover)
	const [cardRecId, setCardRecId] = useState<string | null>(null)
	const cardRecRef = useRef<MediaRecorder | null>(null)
	async function dictateCard(id: string) {
		if (cardRecId === id) {
			cardRecRef.current?.stop()
			return
		}
		if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) {
			setBusy(t('mic.blockedShort'))
			return
		}
		let stream: MediaStream
		try {
			stream = await navigator.mediaDevices.getUserMedia(MIC_CONSTRAINTS)
		} catch (e) {
			setBusy(micErrorMsg(e))
			return
		}
		const chunks: BlobPart[] = []
		const mr = new MediaRecorder(stream, REC_OPTS)
		mr.ondataavailable = (ev) => ev.data.size && chunks.push(ev.data)
		mr.onstop = async () => {
			stream.getTracks().forEach((t) => t.stop())
			setCardRecId(null)
			const type = mr.mimeType || 'audio/webm'
			const ext = type.includes('mp4') ? 'mp4' : type.includes('ogg') ? 'ogg' : 'webm'
			setBusy(t('card.listening'))
			try {
				// AI understands the speech and updates this card's text / tags / owner / kind
				const r = await fetch(`${SYNC_HTTP}/api/card/${encodeURIComponent(room)}/${encodeURIComponent(id)}?ext=${ext}`, {
					method: 'POST',
					headers: { 'Content-Type': type, ...byoHeaders() },
					body: new Blob(chunks, { type }),
				}).then((x) => x.json())
				if (r.ok) {
					showSubtitle(r.transcript)
					const parts: string[] = []
					if (r.edit?.text !== undefined) parts.push(t('card.partText'))
					if (r.edit?.tags) parts.push(t('card.partTags'))
					if (r.edit?.owner !== undefined) parts.push(t('card.partOwner'))
					if (r.edit?.color) parts.push(t('card.partKind'))
					setBusy(parts.length ? t('card.updated', { parts: parts.join(t('card.partsJoin')) }) : r.transcript ? t('card.nothingToChange') : t('card.nothingHeard'))
				} else setBusy(r.error ? t('agent.error', { message: r.error }) : t('agent.errorPlain'))
			} catch (e) {
				setBusy(t('agent.error', { message: (e as Error).message }))
			}
		}
		cardRecRef.current = mr
		mr.start()
		setCardRecId(id)
		setBusy(t('card.speakNow'))
	}

	// continuous "meeting" mode: keep listening, auto-cut a segment on each pause
	// (silence) and send it for transcription+agent, so cards appear hands-free.
	const [meeting, setMeeting] = useState(false)
	const [segCount, setSegCount] = useState(0) // 誠實計數:只算「成功上板」的段
	const meetingRef = useRef<{ stop: () => void } | null>(null)
	// 語音段落絕不無聲丟失:送出中/失敗都看得到,失敗的 blob 留著可重送
	const [pendingSegs, setPendingSegs] = useState(0)
	const [failedSegs, setFailedSegs] = useState<number[]>([])
	const failedBlobs = useRef(new Map<number, Blob>())
	const segSeq = useRef(0)
	const pausedUntil = useRef(0) // 被 demo 限流時暫停送出(epoch ms)
	const pauseTimer = useRef<any>(null)
	const sendQueue = useRef<{ id: number; blob: Blob }[]>([])
	const meterRef = useRef(0) // 即時 RMS,給 VolBars 輪詢(避免 120ms setState 重繪整個 App)
	// 手機錄音韌性:螢幕喚醒鎖(熄屏會殺錄音)+ 系統收回麥克風時的恢復提示
	const wakeLockRef = useRef<any>(null)
	const [recInterrupted, setRecInterrupted] = useState(false)
	async function acquireWakeLock() {
		try {
			wakeLockRef.current = await (navigator as any).wakeLock?.request('screen')
		} catch {} // 不支援或被拒就算了,錄音照跑
	}
	function releaseWakeLock() {
		try {
			wakeLockRef.current?.release()
		} catch {}
		wakeLockRef.current = null
	}
	useEffect(() => {
		if (!meeting) return
		// 切去別的 app 再回來:系統會釋放 wake lock,要重新拿
		const onVis = () => {
			if (document.visibilityState === 'visible' && meeting) void acquireWakeLock()
		}
		document.addEventListener('visibilitychange', onVis)
		return () => document.removeEventListener('visibilitychange', onVis)
	}, [meeting])
	// 語音上傳只需要單聲道人聲:壓低位元率(預設 ~128kbps → 48kbps),行動網路上傳快、流量省
	const REC_OPTS: MediaRecorderOptions = { audioBitsPerSecond: 48000 }
	const MIC_CONSTRAINTS: MediaStreamConstraints = {
		audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
	}

	function micErrorMsg(e: any): string {
		const n = e?.name || ''
		if (n === 'NotAllowedError' || n === 'PermissionDeniedError') return t('mic.denied')
		if (n === 'NotFoundError' || n === 'DevicesNotFoundError') return t('mic.notFound')
		if (n === 'NotReadableError' || n === 'TrackStartError') return t('mic.busy')
		return t('mic.generic', { detail: n || e?.message || e })
	}

	function queueSegment(blob: Blob) {
		if (!blob.size) return
		const id = ++segSeq.current
		if (Date.now() < pausedUntil.current) {
			sendQueue.current.push({ id, blob })
			setPendingSegs((n) => n + 1)
			return
		}
		void sendSegment(id, blob, 0)
	}
	function failSeg(id: number, blob: Blob, why: string) {
		failedBlobs.current.set(id, blob)
		setFailedSegs((a) => (a.includes(id) ? a : [...a, id]))
		setBusy(t('rec.segFailed', { why }))
	}
	// 限流暫停:倒數結束自動把佇列裡的段依序補送(間隔 1.2s,避免又撞限流)
	function schedulePauseResume(wait: number) {
		clearTimeout(pauseTimer.current)
		setBusy(t('rec.rateLimited', { wait }))
		pauseTimer.current = setTimeout(async () => {
			const q = sendQueue.current.splice(0)
			setPendingSegs((n) => Math.max(0, n - q.length))
			for (const s of q) {
				await sendSegment(s.id, s.blob, 0)
				await new Promise((r) => setTimeout(r, 1200))
			}
		}, wait * 1000)
	}
	async function sendSegment(id: number, blob: Blob, attempt: number): Promise<void> {
		const type = blob.type || 'audio/webm'
		const ext = type.includes('mp4') ? 'mp4' : type.includes('ogg') ? 'ogg' : 'webm'
		setPendingSegs((n) => n + 1)
		try {
			const res = await fetch(`${SYNC_HTTP}/api/voice/${encodeURIComponent(room)}?ext=${ext}&by=${encodeURIComponent(me.name)}`, {
				method: 'POST',
				headers: { 'Content-Type': type, ...byoHeaders() },
				body: blob,
			})
			const r =
				res.status === 429
					? { ok: false, rateLimited: true, retryAfterSeconds: parseInt(res.headers.get('Retry-After') || '15', 10) || 15 }
					: await res.json()
			setPendingSegs((n) => Math.max(0, n - 1))
			if (r.rateLimited) {
				const wait = r.retryAfterSeconds || 15
				pausedUntil.current = Date.now() + wait * 1000
				sendQueue.current.push({ id, blob })
				setPendingSegs((n) => n + 1)
				schedulePauseResume(wait)
				return
			}
			if (!r.ok) {
				failSeg(id, blob, r.error || t('rec.serverError'))
				return
			}
			setSegCount((c) => c + 1)
			showSubtitle(r.transcript) // UX: let the speaker see what was heard
			logTranscript(r.transcript) // keep the word-for-word meeting log
			applyAgentResponse(r) // a segment may be a spoken command, not content
		} catch {
			setPendingSegs((n) => Math.max(0, n - 1))
			if (attempt < 1) {
				// 網路抖一下先自動重試一次
				setTimeout(() => void sendSegment(id, blob, attempt + 1), 3000)
			} else {
				failSeg(id, blob, t('rec.networkDown'))
			}
		}
	}
	async function retryFailedSegs() {
		const ids = [...failedSegs]
		setFailedSegs([])
		for (const id of ids) {
			const blob = failedBlobs.current.get(id)
			failedBlobs.current.delete(id)
			if (blob) {
				await sendSegment(id, blob, 1)
				await new Promise((r) => setTimeout(r, 800))
			}
		}
	}

	async function startMeeting() {
		if (maybeAskName()) return // 補問名字(卡片要標「誰提的」);填完或再按一次就開錄
		if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) {
			setBusy(t('mic.blockedMeeting', { origin: `${location.protocol}//${location.host}` }))
			return
		}
		let stream: MediaStream
		try {
			stream = await navigator.mediaDevices.getUserMedia(MIC_CONSTRAINTS)
		} catch (e) {
			setBusy(micErrorMsg(e))
			return
		}
		setMeeting(true)
		setSegCount(0)
		setRecInterrupted(false)
		void acquireWakeLock() // 錄音中不讓螢幕自動熄(熄屏 = 系統殺錄音)
		setBusy(t('rec.meetingBusy'))

		const ctx = new AudioContext()
		const analyser = ctx.createAnalyser()
		analyser.fftSize = 1024
		ctx.createMediaStreamSource(stream).connect(analyser)
		const buf = new Uint8Array(analyser.fftSize)

		const SPEAK = 0.018 // RMS threshold for "speech"
		const SILENCE_MS = 1200 // pause length that ends a segment
		const MIN_MS = 1500 // ignore ultra-short blips
		const MAX_MS = 25000 // force a cut on long monologues
		const NO_VOICE_WARN_MS = 9000 // 開錄這麼久都沒聲音 → 提醒檢查輸入裝置

		let mr: MediaRecorder | null = null
		let chunks: BlobPart[] = []
		let segStart = 0
		let spoke = false
		let silentSince = 0
		let alive = true
		let lastVoice = performance.now()
		let warnedSilence = false

		const startSeg = () => {
			chunks = []
			mr = new MediaRecorder(stream, REC_OPTS)
			mr.ondataavailable = (ev) => ev.data.size && chunks.push(ev.data)
			mr.onstop = () => queueSegment(new Blob(chunks, { type: mr?.mimeType || 'audio/webm' }))
			mr.start()
			segStart = performance.now()
			spoke = false
			silentSince = 0
		}
		const cut = () => {
			try {
				if (mr && mr.state !== 'inactive') mr.stop() // onstop sends this segment
			} catch {}
			setBusy(t('rec.segmentCut'))
			if (alive) startSeg()
		}
		const iv = setInterval(() => {
			if (!alive) return
			// 手機切走/熄屏後系統可能 suspend AudioContext、停掉 MediaRecorder、甚至收回麥克風:
			// 能自動救的就地救,救不了(track 死了)就亮「恢復」鈕
			const track = stream.getAudioTracks()[0]
			if (!track || track.readyState === 'ended') {
				setRecInterrupted(true)
				return
			}
			if (ctx.state === 'suspended') void ctx.resume().catch(() => {})
			if (mr && mr.state === 'inactive') startSeg()
			analyser.getByteTimeDomainData(buf)
			let sum = 0
			for (let i = 0; i < buf.length; i++) {
				const v = (buf[i] - 128) / 128
				sum += v * v
			}
			const rms = Math.sqrt(sum / buf.length)
			meterRef.current = rms // 即時音量,VolBars 自己輪詢,不經 React state
			const now = performance.now()
			if (rms > SPEAK) {
				spoke = true
				silentSince = 0
				lastVoice = now
				warnedSilence = false
			} else if (spoke && !silentSince) {
				silentSince = now
			}
			if (!warnedSilence && now - lastVoice > NO_VOICE_WARN_MS) {
				warnedSilence = true
				setBusy(t('rec.noVoice'))
			}
			const dur = now - segStart
			if ((spoke && silentSince && now - silentSince > SILENCE_MS && dur > MIN_MS) || (spoke && dur > MAX_MS)) cut()
		}, 120)

		meetingRef.current = {
			stop: () => {
				alive = false
				clearInterval(iv)
				try {
					if (mr && mr.state !== 'inactive') mr.stop() // flush final segment
				} catch {}
				stream.getTracks().forEach((t) => t.stop())
				ctx.close().catch(() => {})
				releaseWakeLock()
				setRecInterrupted(false)
				setMeeting(false)
				setBusy(t('rec.meetingDone'))
			},
		}
		startSeg()
	}
	function stopMeeting() {
		meetingRef.current?.stop()
	}
	async function toggleRecord() {
		if (recording) {
			recRef.current?.stop()
			return
		}
		// Mic needs a SECURE context (https or localhost). A plain http LAN IP like
		// http://192.168.x.y is blocked by the browser → mediaDevices is undefined.
		if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) {
			setBusy(t('mic.blockedRecord', { origin: `${location.protocol}//${location.host}` }))
			return
		}
		let stream: MediaStream
		try {
			stream = await navigator.mediaDevices.getUserMedia(MIC_CONSTRAINTS)
		} catch (e) {
			setBusy(micErrorMsg(e))
			return
		}
		const chunks: BlobPart[] = []
		const mr = new MediaRecorder(stream, REC_OPTS)
		mr.ondataavailable = (ev) => ev.data.size && chunks.push(ev.data)
		mr.onstop = async () => {
			stream.getTracks().forEach((tr) => tr.stop())
			setRecording(false)
			setBusy(t('rec.transcribing'))
			// iOS Safari often records mp4/aac, not webm — use the actual mimeType so
			// the file extension matches (mori-ear/Groq Whisper accept mp4/m4a/ogg/webm).
			const type = mr.mimeType || 'audio/webm'
			const ext = type.includes('mp4') ? 'mp4' : type.includes('ogg') ? 'ogg' : 'webm'
			const blob = new Blob(chunks, { type })
			try {
				const r = await fetch(
					`${SYNC_HTTP}/api/voice/${encodeURIComponent(room)}?ext=${ext}&by=${encodeURIComponent(me.name)}`,
					{ method: 'POST', headers: { 'Content-Type': type, ...byoHeaders() }, body: blob }
				).then((x) => x.json())
				showSubtitle(r.transcript)
				logTranscript(r.transcript)
				applyAgentResponse(r, r.transcript ? t('rec.heardPrefix', { text: r.transcript }) : '')
			} catch (e) {
				setBusy(t('agent.error', { message: (e as Error).message }))
			}
		}
		recRef.current = mr
		mr.start()
		setRecording(true)
		setBusy(t('rec.recordingOnce'))
	}

	const mobile = size.w < 700
	// PWA install affordance — only when it makes sense (not already installed, not the Tauri app)
	const isStandalone = window.matchMedia('(display-mode: standalone)').matches || (navigator as any).standalone === true
	const isIOS = /iphone|ipad|ipod/i.test(navigator.userAgent)
	const isTauriApp = '__TAURI_INTERNALS__' in window || '__TAURI__' in window
	const canInstall = !isTauriApp && !isStandalone && (!!installPrompt || (isIOS && /safari/i.test(navigator.userAgent) && !/crios|fxios/i.test(navigator.userAgent)))
	const doInstall = async () => {
		setMenuOpen(false)
		if (installPrompt) {
			installPrompt.prompt()
			try {
				await installPrompt.userChoice
			} catch {}
			setInstallPrompt(null)
		} else if (isIOS) setIosInstallHint(true)
	}
	// bare <button> is styled globally (index.html); keep this empty so variant
	// overrides (background, width…) layer on top cleanly.
	const btn: React.CSSProperties = {}

	// exposed for verification / console poking
	;(window as any).__wb = {
		addSticky,
		patchShape,
		deleteSticky,
		addConnector,
		clearAll,
		cmd: async (t: string) =>
			applyAgentResponse(
				await fetch(`${SYNC_HTTP}/api/agent/${encodeURIComponent(room)}`, {
					method: 'POST',
					headers: { 'Content-Type': 'application/json', ...byoHeaders() },
					body: JSON.stringify({ transcript: t, by: me.name }),
				}).then((x) => x.json())
			),
	}
	;(window as any).__cursors = cursors

	return (
		<div className="board-bg" style={{ position: 'fixed', inset: 0 }}>
			{/* name gate — ask for a real name before joining (shows over everything) */}
			{needName && (
				<div className="scrim" style={{ zIndex: 3800 }}>
					<div className="dialog-card modal-in" style={{ width: 'min(380px, 92vw)', textAlign: 'center' }}>
						<div style={{ fontFamily: 'Fraunces, serif', fontSize: 26, lineHeight: 1, marginBottom: 8, color: 'var(--accent)' }}>Mori Canvas</div>
						<div style={{ fontWeight: 700, fontSize: 18 }}>{t('nameGate.welcome')}</div>
						<div className="muted" style={{ fontSize: 13, margin: '6px 0 16px' }}>{t('nameGate.why')}</div>
						<input
							autoFocus
							value={nameDraft}
							onChange={(e) => setNameDraft(e.target.value.slice(0, 24))}
							onKeyDown={(e) => {
								if (e.key === 'Enter' && nameDraft.trim()) {
									setMyName(nameDraft.trim())
									setNeedName(false)
								}
							}}
							placeholder={t('nameGate.placeholder')}
							style={{ width: '100%', fontSize: 16, padding: '10px 12px', textAlign: 'center' }}
						/>
						<button
							className="btn-accent"
							disabled={!nameDraft.trim()}
							style={{ width: '100%', marginTop: 12, padding: '11px', fontSize: 15, fontWeight: 600 }}
							onClick={() => {
								setMyName(nameDraft.trim())
								setNeedName(false)
							}}
						>
							{t('nameGate.enter')}
						</button>
						<button
							style={{ width: '100%', marginTop: 8, padding: '9px', fontSize: 13, background: 'transparent', border: 'none', color: 'var(--ink-soft)', cursor: 'pointer', textDecoration: 'underline' }}
							onClick={() => {
								// 先逛再說:用訪客身分進場;第一次錄音/建卡/分享時才會再問一次
								setNeedName(false)
							}}
						>
							{t('nameGate.justLook')}
						</button>
					</div>
				</div>
			)}

			{/* first-run onboarding / help */}
			{guide && !needName && (
				<div
					style={{
						position: 'fixed',
						inset: 0,
						zIndex: 4000,
						background: 'rgba(28,26,23,0.5)',
						backdropFilter: 'blur(4px)',
						display: 'flex',
						alignItems: 'center',
						justifyContent: 'center',
						padding: 16,
					}}
				>
					<div
						className="glass modal-in"
						style={{ background: 'var(--surface)', width: 'min(440px, 92vw)', maxHeight: '88vh', overflowY: 'auto', padding: '26px 24px 20px', borderRadius: 20 }}
					>
						<div className="code" style={{ fontSize: 30, color: 'var(--ink)' }}>{t('guide.title')}</div>
						<div style={{ color: 'var(--ink-soft)', fontSize: 14, margin: '2px 0 18px' }}>{t('guide.subtitle')}</div>
						{(
							[
								[t('guide.step1Title'), t('guide.step1Body')],
								[t('guide.step2Title'), t('guide.step2Body')],
								[t('guide.step3Title'), '__LEGEND__'],
								[t('guide.step4Title'), t('guide.step4Body')],
								[t('guide.step5Title'), t('guide.step5Body')],
								[t('guide.step6Title'), t('guide.step6Body')],
							] as [string, string][]
						).map(([title, d], i) => (
							<div key={i} style={{ display: 'flex', gap: 12, marginBottom: 14 }}>
								<div style={{ flex: '0 0 24px', height: 24, borderRadius: '50%', background: 'var(--accent)', color: '#fff', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 13, fontWeight: 600 }}>
									{i + 1}
								</div>
								<div style={{ flex: 1 }}>
									<div style={{ fontWeight: 600, fontSize: 14, marginBottom: 3 }}>{title}</div>
									{d === '__LEGEND__' ? (
										<div style={{ display: 'flex', gap: 14, flexWrap: 'wrap', fontSize: 13, color: 'var(--ink-soft)' }}>
											{KIND_ORDER.map((c) => (
												<span key={c} style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
													<span style={{ width: 13, height: 13, borderRadius: 4, background: COLORS[c], display: 'inline-block' }} />
													{kindLabel(c)}
												</span>
											))}
										</div>
									) : (
										<div style={{ fontSize: 13, color: 'var(--ink-soft)', lineHeight: 1.55 }}>{d}</div>
									)}
								</div>
							</div>
						))}
						<div style={{ display: 'flex', gap: 8, marginTop: 6 }}>
							<button style={{ flex: 1, padding: '9px' }} onClick={() => openExamples()}>
								{t('guide.openExamples')}
							</button>
							<button
								style={{ flex: 1, padding: '9px' }}
								onClick={() => {
									localStorage.setItem('wb-seen-guide', '1')
									setGuide(false)
									setTourStep(0)
								}}
							>
								{t('guide.runTour')}
							</button>
						</div>
						<button
							className="btn-accent" style={{ width: '100%', marginTop: 8, padding: '11px', fontSize: 15, fontWeight: 600 }}
							onClick={() => {
								const first = !localStorage.getItem('wb-seen-guide')
								localStorage.setItem('wb-seen-guide', '1')
								setGuide(false)
								if (first && !localStorage.getItem('wb-tour-done')) setTourStep(0)
							}}
						>
							{t('guide.start')}
						</button>
					</div>
				</div>
			)}

			{/* 範例庫:persona 範例板,載入看成品、照著講法示範自己講一遍 */}
			{examplesOpen && (
				<div
					style={{ position: 'fixed', inset: 0, zIndex: 4100, background: 'rgba(28,26,23,0.5)', backdropFilter: 'blur(4px)', display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 16 }}
					onClick={() => setExamplesOpen(false)}
				>
					<div
						className="glass modal-in"
						style={{ background: 'var(--surface)', width: 'min(560px, 94vw)', maxHeight: '86vh', overflowY: 'auto', padding: '24px 22px 18px', borderRadius: 20 }}
						onClick={(e) => e.stopPropagation()}
					>
						<div className="code" style={{ fontSize: 24, color: 'var(--ink)' }}>{t('examples.title')}</div>
						<div style={{ color: 'var(--ink-soft)', fontSize: 13, margin: '2px 0 14px' }}>
							{t('examples.subtitle')}
						</div>
						{/* 範例是「內容」不是 UI,保持繁中;en 介面加一行說明 */}
						{t('examples.zhNote') && (
							<div className="muted" style={{ fontSize: 12, margin: '0 0 12px', border: '1px solid var(--line)', borderRadius: 10, padding: '7px 10px' }}>
								{t('examples.zhNote')}
							</div>
						)}
						{!exampleIndex.length && <div className="muted" style={{ fontSize: 13 }}>{t('examples.loadingIndex')}</div>}
						{(() => {
							// 內建範例與社群範本共用同一張卡片(只差載入來源目錄)
							const boardCard = (ex: any, dir: 'examples' | 'templates') => (
								<div key={`${dir}/${ex.id}`} style={{ border: '1px solid var(--line)', borderRadius: 14, padding: '12px 14px', marginBottom: 10 }}>
									<div style={{ display: 'flex', alignItems: 'flex-start', gap: 10 }}>
										<div style={{ flex: 1, minWidth: 0 }}>
											<div style={{ fontWeight: 600, fontSize: 14 }}>
												{ex.persona} · {ex.title}
											</div>
											<div style={{ fontSize: 12.5, color: 'var(--ink-soft)', margin: '3px 0 6px' }}>{ex.blurb}</div>
											<div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
												{(ex.boards || []).map((b: string) => (
													<span key={b} className="muted" style={{ fontSize: 11, border: '1px solid var(--line)', borderRadius: 999, padding: '2px 9px' }}>{b}</span>
												))}
											</div>
										</div>
										<div style={{ flex: '0 0 auto', display: 'flex', flexDirection: 'column', gap: 6 }}>
											<button className="btn-accent" style={{ padding: '7px 16px' }} onClick={() => loadExample(ex.id, dir)}>
												{t('examples.load')}
											</button>
											<button style={{ padding: '5px 10px', fontSize: 11.5 }} title={t('examples.copyLinkTitle')} onClick={() => copyBoardLink(ex.id)}>
												{t('examples.copyLink')}
											</button>
										</div>
									</div>
									{(ex.sampleUtterances || []).length > 0 && (
										<details style={{ marginTop: 8 }}>
											<summary style={{ cursor: 'pointer', fontSize: 12, color: 'var(--accent)' }}>{t('examples.utterances')}</summary>
											<ul style={{ margin: '6px 0 0', paddingLeft: 18, fontSize: 12.5, color: 'var(--ink-soft)', lineHeight: 1.7 }}>
												{ex.sampleUtterances.map((u: string, i: number) => (
													<li key={i}>「{u}」</li>
												))}
											</ul>
										</details>
									)}
								</div>
							)
							return (
								<>
									{exampleIndex.map((ex: any) => boardCard(ex, 'examples'))}
									{templateIndex.length > 0 && (
										<>
											<div className="code" style={{ fontSize: 17, color: 'var(--ink)', margin: '14px 0 2px' }}>{t('examples.community')}</div>
											<div style={{ color: 'var(--ink-soft)', fontSize: 12, margin: '0 0 10px' }}>
												<Trans
													i18nKey="examples.communityNote"
													components={[<a key="0" href="https://github.com/yazelin/mori-canvas/blob/main/client/public/templates/README.md" target="_blank" rel="noreferrer" style={{ color: 'var(--accent)' }} />]}
												/>
											</div>
											{templateIndex.map((ex: any) => boardCard(ex, 'templates'))}
										</>
									)}
								</>
							)
						})()}
						<div className="muted" style={{ fontSize: 11.5 }}>{t('examples.overwriteNote')}</div>
					</div>
				</div>
			)}

			{/* 互動導覽:spotlight 逐步指向真實按鈕(選不到的步驟自動跳過,如手機版) */}
			{tourStep >= 0 &&
				(() => {
					const steps = TOUR_STEPS.filter((s) => document.querySelector(s.sel))
					if (!steps.length) return null
					const i = Math.min(tourStep, steps.length - 1)
					const el = document.querySelector(steps[i].sel)!
					const r = el.getBoundingClientRect()
					const pad = 6
					const below = r.bottom + 170 < window.innerHeight
					const tipTop = below ? r.bottom + pad + 12 : undefined
					const tipBottom = below ? undefined : window.innerHeight - r.top + pad + 12
					const tipLeft = Math.max(10, Math.min(r.left, window.innerWidth - 330))
					const done = () => {
						localStorage.setItem('wb-tour-done', '1')
						setTourStep(-1)
					}
					return (
						<div style={{ position: 'fixed', inset: 0, zIndex: 4200 }} onClick={done}>
							<div
								style={{
									position: 'fixed',
									left: r.left - pad,
									top: r.top - pad,
									width: r.width + pad * 2,
									height: r.height + pad * 2,
									borderRadius: 14,
									border: '2px solid var(--accent)',
									boxShadow: '0 0 0 9999px rgba(28,26,23,0.55)',
									pointerEvents: 'none',
								}}
							/>
							<div
								className="glass modal-in"
								style={{ position: 'fixed', left: tipLeft, top: tipTop, bottom: tipBottom, zIndex: 4201, background: 'var(--surface)', width: 'min(320px, calc(100vw - 20px))', padding: '14px 16px', borderRadius: 14 }}
								onClick={(e) => e.stopPropagation()}
							>
								<div style={{ fontWeight: 600, fontSize: 14, marginBottom: 4 }}>
									{t(`tour.${steps[i].key}.title`)}
									<span className="muted" style={{ fontWeight: 400, fontSize: 11.5, marginLeft: 8 }}>{i + 1} / {steps.length}</span>
								</div>
								<div style={{ fontSize: 13, color: 'var(--ink-soft)', lineHeight: 1.6 }}>{t(`tour.${steps[i].key}.body`)}</div>
								<div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
									<button style={{ padding: '6px 12px', fontSize: 12.5 }} onClick={done}>{t('tour.skip')}</button>
									<span style={{ flex: 1 }} />
									{i > 0 && (
										<button style={{ padding: '6px 12px', fontSize: 12.5 }} onClick={() => setTourStep(i - 1)}>{t('tour.prev')}</button>
									)}
									{i < steps.length - 1 ? (
										<button className="btn-accent" style={{ padding: '6px 14px', fontSize: 12.5 }} onClick={() => setTourStep(i + 1)}>{t('tour.next')}</button>
									) : (
										<button className="btn-accent" style={{ padding: '6px 14px', fontSize: 12.5 }} onClick={done}>{t('tour.done')}</button>
									)}
								</div>
							</div>
						</div>
					)
				})()}
			<Stage
				ref={stageRef}
				width={size.w}
				height={size.h}
				x={view.x}
				y={view.y}
				scaleX={view.scale}
				scaleY={view.scale}
				draggable
				onWheel={onWheel}
				onDragMove={(e: any) => {
					if (e.target === e.target.getStage()) setView((v) => ({ ...v, x: e.target.x(), y: e.target.y() }))
				}}
				onDragEnd={(e: any) => {
					if (e.target === e.target.getStage()) setView((v) => ({ ...v, x: e.target.x(), y: e.target.y() }))
				}}
				onMouseDown={(e: any) => {
					if (e.target === e.target.getStage()) {
						setSelectedId(null)
						setSelectedConnId(null)
						setConnectFrom(null)
					}
				}}
				onMouseMove={publishCursor}
				onMouseLeave={clearCursor}
				onTouchMove={onTouchMove}
				onTouchEnd={onTouchEnd}
				onDblClick={onStageDblClick}
				onDblTap={onStageDblClick}
			>
				<Layer>
					{/* diagram frames (behind everything) — each is a typed sub-board */}
					{frames.map((f) => (
						<Group key={f.id}>
							<Rect
								x={f.x}
								y={f.y}
								width={f.w}
								height={f.h}
								cornerRadius={18}
								fill={ct.frameFill}
								stroke={ct.frameStroke}
								strokeWidth={1.5}
								listening={false}
								{...PERF}
							/>
							{/* title bar = drag handle (moves frame + its cards); double-click to rename */}
							<Rect
								x={f.x}
								y={f.y}
								width={f.w}
								height={34}
								cornerRadius={[18, 18, 0, 0]}
								fill={ct.frameHeader}
								{...PERF}
								draggable={!READ_ONLY}
								onDragMove={(e: any) => {
									const dx = e.target.x() - f.x
									const dy = e.target.y() - f.y
									if (Math.abs(dx) > 0.5 || Math.abs(dy) > 0.5) moveFrame(f, dx, dy)
									e.target.position({ x: f.x, y: f.y })
								}}
								onDblClick={(e: any) => {
									e.cancelBubble = true
									setEditingFrame({ id: f.id, value: f.title || '' })
								}}
								onMouseEnter={(e: any) => (e.target.getStage().container().style.cursor = 'move')}
								onMouseLeave={(e: any) => (e.target.getStage().container().style.cursor = 'default')}
							/>
							<Text
								x={f.x + 16}
								y={f.y + 10}
								text={`${typeLabel(f.type)}　${f.title || ''}`}
								fontSize={14}
								fontStyle="600"
								fontFamily={CANVAS_FONT}
								fill={ct.frameTitle}
								listening={false}
								{...PERF}
							/>
							{/* delete this whole diagram (frame + its cards) */}
							<Group
								x={f.x + f.w - 30}
								y={f.y + 7}
								onClick={(e: any) => {
									e.cancelBubble = true
									deleteFrame(f.id)
								}}
								onTap={(e: any) => {
									e.cancelBubble = true
									deleteFrame(f.id)
								}}
								onMouseEnter={(e: any) => (e.target.getStage().container().style.cursor = 'pointer')}
								onMouseLeave={(e: any) => (e.target.getStage().container().style.cursor = 'default')}
							>
								<Rect width={22} height={20} cornerRadius={7} fill={ct.frameHeader} {...PERF} />
								<Text width={22} height={20} text="✕" fontSize={13} fontFamily={CANVAS_FONT} fill={ct.frameTitle} align="center" verticalAlign="middle" listening={false} {...PERF} />
							</Group>
							{/* resize handle (bottom-right) — themed corner grip */}
							<Rect
								x={f.x + f.w - 20}
								y={f.y + f.h - 20}
								width={14}
								height={14}
								cornerRadius={[8, 4, 8, 4]}
								fill={ct.handle}
								stroke={ct.handleStroke}
								strokeWidth={1.5}
								{...PERF}
								draggable={!READ_ONLY}
								onDragMove={(e: any) => {
									const w = Math.max(280, e.target.x() - f.x + 20)
									const h = Math.max(200, e.target.y() - f.y + 20)
									patchFrame(f.id, { w, h })
									e.target.position({ x: f.x + w - 20, y: f.y + h - 20 })
								}}
								onMouseEnter={(e: any) => (e.target.getStage().container().style.cursor = 'nwse-resize')}
								onMouseLeave={(e: any) => (e.target.getStage().container().style.cursor = 'default')}
							/>
						</Group>
					))}
					{/* connectors behind stickies */}
					{connectors.map((c) => {
						const a = byId(c.from)
						const b = byId(c.to)
						if (!a || !b) return null
						if (!connVisible(c, a, b)) return null // viewport culling
						const ac: [number, number] = [a.x + a.w / 2, a.y + a.h / 2]
						const bc: [number, number] = [b.x + b.w / 2, b.y + b.h / 2]
						const [x1, y1] = edgePoint(ac[0], ac[1], a.w / 2, a.h / 2, bc[0], bc[1])
						const [x2, y2] = edgePoint(bc[0], bc[1], b.w / 2, b.h / 2, ac[0], ac[1])
						const sel = c.id === selectedConnId
						// a connector spanning two different diagrams = a cross-reference, drawn dashed
						const cross = a.frameId && b.frameId && a.frameId !== b.frameId
						const baseColor = cross ? (theme === 'dark' ? 'rgba(167,139,250,0.7)' : 'rgba(124,58,160,0.6)') : ct.conn
						// hierarchical diagrams use orthogonal "axis" elbow lines
						const frame = a.frameId && a.frameId === b.frameId ? frames.find((f) => f.id === a.frameId) : null
						const tdir = frame ? WB_TREE_DIR[frame.type] : null
						const points = tdir && !cross ? elbowPoints(a, b, tdir) : [x1, y1, x2, y2]
						return (
							<Arrow
								key={c.id}
								points={points}
								stroke={sel ? ACCENT : baseColor}
								fill={sel ? ACCENT : baseColor}
								strokeWidth={sel ? 3.5 : 2}
								dash={cross ? [10, 7] : undefined}
								hitStrokeWidth={16}
								pointerLength={10}
								pointerWidth={10}
								tension={0}
								{...PERF}
								onClick={() => {
									setSelectedConnId(c.id)
									setSelectedId(null)
								}}
								onTap={() => {
									setSelectedConnId(c.id)
									setSelectedId(null)
								}}
							/>
						)
					})}
					{shapes.filter(stickyVisible).map((s) => {
						const selected = s.id === selectedId
						const pending = s.id === connectFrom
						return (
							<Group
								key={s.id}
								x={s.x}
								y={s.y}
								draggable={!READ_ONLY}
								opacity={matchesFilter(s) ? 1 : 0.16} onDragStart={() => setSelectedId(s.id)}
								onDragMove={(e: any) => {
									const now = Date.now()
									if (now - dragTs.current < 40) return // throttle yjs writes during drag
									dragTs.current = now
									patchShape(s.id, { x: e.target.x(), y: e.target.y() })
								}}
								onDragEnd={(e: any) => {
										const x = e.target.x()
										const y = e.target.y()
										// dropping a card inside a frame adds it to that diagram (group membership)
										const f = frameAt(x + s.w / 2, y + s.h / 2)
										patchShape(s.id, f ? { x, y, frameId: f.id } : { x, y })
									}}
								onClick={() => onStickyClick(s)}
								onTap={() => onStickyClick(s)}
								onDblClick={(e: any) => {
									e.cancelBubble = true
									if (READ_ONLY) return
									setEditing({ id: s.id, value: s.text })
								}}
							>
								<Rect
									width={s.w}
									height={s.h}
									cornerRadius={16}
									fillLinearGradientStartPoint={{ x: 0, y: 0 }}
									fillLinearGradientEndPoint={{ x: 0, y: s.h }}
									fillLinearGradientColorStops={[0, lighten(cardColor(s.color), theme === 'dark' ? 0.05 : 0.09), 1, cardColor(s.color)]}
									shadowColor="#1c1a17"
									shadowOpacity={selected ? 0.28 : 0.17}
									shadowBlur={selected ? 26 : 18}
									shadowOffsetY={selected ? 12 : 8}
									shadowForStrokeEnabled={false}
									stroke={pending ? ACCENT : selected ? cc.selStroke : cc.stroke}
									strokeWidth={pending ? 3 : selected ? 2 : 1}
									{...PERF}
								/>
								{/* kind accent dot + 類型小標(只在會議板語意下顯示,其他板型同色另有意義) */}
								<Circle x={18} y={18} radius={5} fill={kindAccent(s.color)} opacity={0.8} listening={false} {...PERF} />
								{!(s as any).note && isMeetingFrame(s.frameId) && kindLabel(s.color) && (
									<Text x={28} y={12} text={kindLabel(s.color)} fontSize={10} fontStyle="600" fontFamily={CANVAS_FONT} fill={kindAccent(s.color)} opacity={0.85} listening={false} {...PERF} />
								)}
								{/* editor number (fallback handle: "把 N 號…") — not on notes */}
								{!(s as any).note && cardNum[s.id] && (
									<Text x={s.w - 30} y={11} width={20} align="right" text={String(cardNum[s.id])} fontSize={12} fontStyle="600" fontFamily={CANVAS_FONT} fill={cc.num} listening={false} {...PERF} />
								)}
								<Text
									text={s.text}
									width={s.w}
									height={s.h}
									padding={20}
									fontSize={s.fontSize || BASE_FONT}
									lineHeight={1.25}
									fontFamily={CANVAS_FONT}
									fontStyle="500"
									fill={cc.text}
									align="center"
									verticalAlign="middle"
									listening={false}
									{...PERF}
								/>
								{/* content tags (top row) — click to filter by tag */}
								{(() => {
									let tx = !(s as any).note && isMeetingFrame(s.frameId) && kindLabel(s.color) ? 56 : 32
									return (s.tags || []).slice(0, 2).map((t, i) => {
										const w = t.length * 11 + 12
										const x = tx
										tx += w + 5
										return (
											<Group
												key={'tag' + i}
												x={x}
												y={9}
												onClick={(e: any) => { e.cancelBubble = true; setFilter({ type: 'tag', value: t }) }}
												onTap={(e: any) => { e.cancelBubble = true; setFilter({ type: 'tag', value: t }) }}
											>
												<Rect width={w} height={17} cornerRadius={8} fill={cc.chipBg} {...PERF} />
												<Text x={6} y={3} text={t} fontSize={10.5} fontFamily={CANVAS_FONT} fill={cc.chipText} listening={false} {...PERF} />
											</Group>
										)
									})
								})()}
								{/* person: owner(負責人) or drawnBy(誰提的) — click to filter by person */}
								{(() => {
									const person = s.owner || (s.drawnBy && s.drawnBy !== 'user' ? s.drawnBy : null)
									if (!person) return null
									const w = Math.min(person.length * 12 + 18, s.w - 24)
									return (
										<Group
											x={12}
											y={s.h - 27}
											onClick={(e: any) => { e.cancelBubble = true; setFilter({ type: 'owner', value: person }) }}
											onTap={(e: any) => { e.cancelBubble = true; setFilter({ type: 'owner', value: person }) }}
										>
											<Rect width={w} height={19} cornerRadius={9.5} fill={s.owner ? cc.ownerBg : cc.chipBg} {...PERF} />
											<Text x={9} y={3.5} text={person} fontSize={11} fontFamily={CANVAS_FONT} fontStyle={s.owner ? '600' : 'normal'} fill={s.owner ? cc.ownerText : cc.chipText} listening={false} {...PERF} />
										</Group>
									)
								})()}
							</Group>
						)
					})}
					{/* live cursors of everyone else (Mori + other humans) */}
					{cursors.map((c) => (
						<Group key={c.id} x={c.x} y={c.y} listening={false}>
							<Circle radius={7} fill={c.color} stroke="#fff" strokeWidth={2} {...PERF} />
							<Rect x={11} y={-10} width={c.name.length * 8.5 + 16} height={20} cornerRadius={10} fill={c.color} {...PERF} />
							<Text x={19} y={-6} text={c.name} fontSize={12} fontFamily={CANVAS_FONT} fontStyle="600" fill="#fff" {...PERF} />
						</Group>
					))}
				</Layer>
			</Stage>

			{/* elegant empty state */}
			{shapes.length === 0 && (
				<div
					style={{
						position: 'fixed',
						inset: 0,
						display: 'flex',
						flexDirection: 'column',
						alignItems: 'center',
						justifyContent: 'center',
						pointerEvents: 'none',
						zIndex: 1,
						textAlign: 'center',
						gap: 8,
					}}
				>
					<div className="code" style={{ fontSize: 38, color: 'var(--ink)', opacity: 0.9 }}>{t('empty.title')}</div>
					<div style={{ fontSize: 15, color: 'var(--ink-soft)' }}>{t('empty.line1')}</div>
					<div style={{ fontSize: 13, color: 'var(--ink-soft)', opacity: 0.75 }}>
						{t('empty.line2')}
					</div>
					<button className="btn-soft" style={{ pointerEvents: 'auto', marginTop: 6, padding: '8px 18px', fontSize: 13 }} onClick={() => openExamples()}>
						{t('empty.loadExample')}
					</button>
					<button
						className="btn-soft"
						style={{ pointerEvents: 'auto', padding: '8px 18px', fontSize: 13 }}
						onClick={async () => {
							// 不用麥克風也不用打字:把內建示範逐字稿餵給 agent,看卡片自己長出來
							if (busy === t('agent.demoThinking') || busy === t('agent.thinking')) return // 防連點重複送
							setBusy(t('agent.demoThinking'))
							try {
								const r = await fetch(`${SYNC_HTTP}/api/agent/${encodeURIComponent(room)}`, {
									method: 'POST',
									headers: { 'Content-Type': 'application/json', ...byoHeaders() },
									body: JSON.stringify({ transcript: t('demo.transcript'), by: me.name }),
								}).then((x) => x.json())
								applyAgentResponse(r, t('agent.demoPrefix'))
							} catch (e) {
								setBusy(t('agent.error', { message: (e as Error).message }))
							}
						}}
					>
						{t('empty.tryDemo')}
					</button>
				</div>
			)}

			{/* text editor overlay */}
			{editing &&
				(() => {
					const s = byId(editing.id)
					if (!s) return null
					return (
						<textarea
							ref={editRef}
							value={editing.value}
							onChange={(e) => setEditing({ id: editing.id, value: e.target.value })}
							onBlur={() => {
								// 提交文字時依內容自動調卡高與字級(只影響手動輸入;AI 建卡文字短,走 server 的 200x200)
								const { h, fontSize } = fitCardSize(editing.value, s.w)
								patchShape(editing.id, { text: editing.value, h, fontSize })
								setEditing(null)
							}}
							onKeyDown={(e) => {
								if (e.key === 'Enter' && !e.shiftKey) {
									e.preventDefault()
									;(e.target as HTMLTextAreaElement).blur()
								}
							}}
							style={{
								position: 'fixed',
								// map canvas coords -> screen through the pan/zoom transform
								left: view.x + s.x * view.scale,
								top: view.y + s.y * view.scale,
								width: s.w * view.scale,
								height: s.h * view.scale,
								border: '2px solid #2563eb',
								borderRadius: 6,
								padding: 8,
								boxSizing: 'border-box',
								fontSize: 18 * view.scale,
								fontFamily: 'system-ui',
								textAlign: 'center',
								resize: 'none',
								background: COLORS[s.color] ?? s.color,
								color: '#1f1c18', // sticky paper is always light -> dark text in both themes
								zIndex: 2000,
							}}
						/>
					)
				})()}

			{/* frame title editor */}
			{editingFrame &&
				(() => {
					const f = frames.find((x) => x.id === editingFrame.id)
					if (!f) return null
					return (
						<input
							autoFocus
							value={editingFrame.value}
							onChange={(e) => setEditingFrame({ id: editingFrame.id, value: e.target.value })}
							onBlur={() => {
								patchFrame(editingFrame.id, { title: editingFrame.value.slice(0, 40) })
								setEditingFrame(null)
							}}
							onKeyDown={(e) => {
								if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
								if (e.key === 'Escape') setEditingFrame(null)
							}}
							style={{
								position: 'fixed',
								left: view.x + (f.x + 52) * view.scale,
								top: view.y + (f.y + 6) * view.scale,
								width: Math.max(120, (f.w - 70) * view.scale),
								fontSize: 14 * view.scale,
								// 跟畫布上的標題同字體/字重,編輯時不再「換一張臉」
								fontFamily: CANVAS_FONT,
								fontWeight: 600,
								color: 'var(--ink)',
								padding: '3px 8px',
								border: '2px solid var(--accent)',
								borderRadius: 6,
								zIndex: 2000,
								background: 'var(--surface)',
							}}
						/>
					)
				})()}

			{/* top bar — desktop: centred room bar; mobile: one compact bar + ⋯ overflow menu */}
			{mobile ? (
				<>
					<div className="glass float-in" style={{ position: 'fixed', top: 'calc(8px + env(safe-area-inset-top, 0px))', left: 'calc(8px + env(safe-area-inset-left, 0px))', right: 'calc(8px + env(safe-area-inset-right, 0px))', zIndex: 1000, display: 'flex', alignItems: 'center', gap: 8, padding: '6px 10px', fontSize: 13 }}>
						<span className="code" style={{ fontSize: 17, color: 'var(--accent)' }}>{room}</span>
						{meeting && <span className="rec-dot live" title={t('topbar.recordingTitle')} style={{ marginRight: 0 }} />}
						<button className="btn-accent" style={{ padding: '5px 11px' }} data-tour="share" onClick={() => { maybeAskName(); setShareOpen(true) }}>{t('topbar.shareShort')}</button>
						<span style={{ flex: 1 }} />
						<span className="muted" style={{ fontSize: 11, whiteSpace: 'nowrap' }}>{statusLabel(status)}·{shapes.length}</span>
						<button style={btn} title={t('topbar.more')} onClick={() => setMenuOpen((v) => !v)}>⋯</button>
					</div>
					{menuOpen && (
						<div className="glass float-in" style={{ position: 'fixed', top: 'calc(54px + env(safe-area-inset-top, 0px))', right: 'calc(8px + env(safe-area-inset-right, 0px))', zIndex: 2100, display: 'flex', flexDirection: 'column', gap: 5, padding: 8, minWidth: 156 }} onClick={() => setMenuOpen(false)}>
							{canInstall && (
								<button className="btn-accent" onClick={(e) => { e.stopPropagation(); doInstall() }}>{t('menu.install')}</button>
							)}
							<button className="btn-soft" onClick={() => setExportOpen(true)}>{t('menu.export')}</button>
							<button onClick={() => setSettingsOpen(true)}>{t('menu.settings')}</button>
							<button onClick={() => toggleTheme()}>{theme === 'dark' ? t('menu.themeLight') : t('menu.themeDark')}</button>
							<button onClick={() => setView({ x: 0, y: 0, scale: 1 })}>{t('menu.resetView')}</button>
							<button onClick={() => openExamples()}>{t('menu.examples')}</button>
							<button onClick={() => setGuide(true)}>{t('menu.help')}</button>
							<button className="btn-danger" onClick={() => { if (window.confirm(t('confirm.clearRoom'))) clearAll() }}>{t('menu.clear')}</button>
						</div>
					)}
				</>
			) : (
				<div className="glass float-in" style={bar}>
					<span className="muted" style={{ fontSize: 12 }}>{t('topbar.room')}</span>
					<span className="code" style={{ fontSize: 19, color: 'var(--accent)', marginRight: 2 }}>{room}</span>
					{meeting && <span className="rec-dot live" title={t('topbar.recordingTitle')} style={{ marginRight: 0 }} />}
					{READ_ONLY && <span className="muted" style={{ fontSize: 12, border: '1px solid var(--line)', borderRadius: 999, padding: '1px 9px' }}>{t('topbar.readOnly')}</span>}
					<button title={t('topbar.shareTitle')} className="btn-accent" data-tour="share" onClick={() => { maybeAskName(); setShareOpen(true) }}>{t('topbar.share')}</button>
					<span className="muted" style={{ fontSize: 12 }} title={status === 'synced' ? t('topbar.liveTitle') : statusLabel(status)}>
						{statusLabel(status)} · {t('topbar.cards', { count: shapes.length })}
					</span>
				</div>
			)}

			{/* 錄音被系統中斷(熄屏/切 app 後麥克風被收回)→ 醒目恢復鈕,一鍵重啟錄音 */}
			{recInterrupted && (
				<div
					className="glass"
					style={{ position: 'fixed', top: 58, left: 0, right: 0, marginInline: 'auto', width: 'fit-content', maxWidth: '92vw', zIndex: 1001, padding: '8px 16px', fontSize: 13, display: 'flex', gap: 10, alignItems: 'center', border: '1px solid var(--live)' }}
				>
					<span style={{ color: 'var(--live)', fontWeight: 600 }}>{t('rec.interrupted')}</span>
					<button
						className="btn-accent"
						style={{ padding: '4px 14px', fontSize: 12.5 }}
						onClick={() => {
							setRecInterrupted(false)
							stopMeeting()
							void startMeeting()
						}}
					>
						{t('rec.resume')}
					</button>
				</div>
			)}

			{/* 連線狀態 banner:斷線立即說明(yjs 會 queue 編輯,連回自動同步);連線中卡 5 秒以上 = 多半是免費主機冷啟動 */}
			{(status === 'disconnected' || (status !== 'synced' && connSlow)) && (
				<div
					className="glass"
					style={{ position: 'fixed', top: 58, left: 0, right: 0, marginInline: 'auto', width: 'fit-content', maxWidth: '92vw', zIndex: 999, padding: '6px 16px', fontSize: 12.5, color: 'var(--ink-soft)' }}
				>
					{status === 'disconnected' ? t('conn.disconnected') : t('conn.coldStart')}
				</div>
			)}

			{/* Ctrl+F 卡片搜尋列(固定頂部置中,Esc 關閉) */}
			{searchOpen && (
				<div
					className="glass float-in"
					style={{ position: 'fixed', top: mobile ? 'calc(52px + env(safe-area-inset-top, 0px))' : 64, left: 0, right: 0, marginInline: 'auto', width: 'fit-content', maxWidth: '94vw', zIndex: 2300, display: 'flex', gap: 7, alignItems: 'center', padding: '6px 10px', fontSize: 13 }}
				>
					<input
						ref={searchInputRef}
						autoFocus
						value={searchQ}
						onChange={(e) => setSearchQ(e.target.value)}
						onKeyDown={(e) => {
							if (e.key === 'Enter') {
								e.preventDefault()
								focusHit(searchIdx + (e.shiftKey ? -1 : 1))
							}
						}}
						placeholder={t('search.placeholder')}
						style={{ width: mobile ? 150 : 210, fontSize: 13, padding: '5px 9px' }}
					/>
					<span className="muted" style={{ fontSize: 12, whiteSpace: 'nowrap', minWidth: 60, textAlign: 'center' }}>
						{searchHits.length ? t('search.hits', { current: Math.min(searchIdx, searchHits.length - 1) + 1, total: searchHits.length }) : searchQ.trim() ? t('search.noHits') : ''}
					</span>
					<button style={{ padding: '3px 9px' }} title={t('search.prevTitle')} disabled={!searchHits.length} onClick={() => focusHit(searchIdx - 1)}>↑</button>
					<button style={{ padding: '3px 9px' }} title={t('search.nextTitle')} disabled={!searchHits.length} onClick={() => focusHit(searchIdx + 1)}>↓</button>
					<button style={{ padding: '3px 9px' }} title={t('search.closeTitle')} onClick={closeSearch}>✕</button>
				</div>
			)}

			{/* canvas tools (left strip, Photoshop-style) */}
			{!READ_ONLY && (<div className="toolstrip float-in">
				<button className="tool" title={t('tools.stickyTitle')} onClick={() => addSticky(140, 140, '', 'yellow') && undefined}><Ico><path d="M16 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h9l6-6V5a2 2 0 0 0-2-2Z"/><path d="M14 21v-5a1 1 0 0 1 1-1h5"/></Ico>{t('tools.sticky')}</button>
				<button className="tool" title={t('tools.noteTitle')} style={{ background: COLORS.note, borderColor: KIND_ACCENT.note, color: '#4a3a6e' }} onClick={() => addNote(180, 180) && undefined}><Ico><path d="M3 11.5V5a2 2 0 0 1 2-2h6.5a2 2 0 0 1 1.4.6l7.5 7.5a2 2 0 0 1 0 2.8l-6.6 6.6a2 2 0 0 1-2.8 0L3.6 12.9A2 2 0 0 1 3 11.5Z"/><circle cx="7.5" cy="7.5" r="1.1" fill="currentColor" stroke="none"/></Ico>{t('tools.note')}</button>
				<button className="tool" title={t('tools.newFrameTitle')} onClick={() => setTypePickerOpen(true)}><Ico><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 8v8M8 12h8"/></Ico>{t('tools.newFrame')}{frames.length ? `·${frames.length}` : ''}</button>
				<div className="tool-divider" />
				<button className={`tool${connectMode ? ' on' : ''}`} title={t('tools.connectTitle')} onClick={() => { setConnectMode((v) => !v); setConnectFrom(null) }}><Ico><path d="M9 17H7A5 5 0 0 1 7 7h2"/><path d="M15 7h2a5 5 0 0 1 0 10h-2"/><line x1="8" y1="12" x2="16" y2="12"/></Ico>{connectMode ? t('tools.connectPending') : t('tools.connect')}</button>
				<button className="tool" title={t('tools.tidyTitle')} onClick={tidy}><Ico><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/></Ico>{t('tools.tidy')}</button>
				<div className="tool-divider" />
				<button className="tool" title={t('tools.undoTitle')} onClick={() => undoMgr.undo()}><Ico><path d="M9 14 4 9l5-5"/><path d="M4 9h11a5 5 0 0 1 5 5 5 5 0 0 1-5 5h-4"/></Ico>{t('tools.undo')}</button>
				<button className="tool" title={t('tools.redoTitle')} onClick={() => undoMgr.redo()}><Ico><path d="m15 14 5-5-5-5"/><path d="M20 9H9a5 5 0 0 0-5 5 5 5 0 0 0 5 5h4"/></Ico>{t('tools.redo')}</button>
				<button className="tool" disabled={!selectedId && !selectedConnId} title={t('tools.deleteTitle')} onClick={() => { if (selectedId) deleteSticky(selectedId); else if (selectedConnId) deleteConnector(selectedConnId) }}><Ico><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></Ico>{t('tools.delete')}</button>
			</div>)}

			{/* app / view (top-right) — desktop only; on mobile these live in the ⋯ menu */}
			{!mobile && (
				<div className="glass float-in" style={appbar}>
					<button title={t('appbar.exportTitle')} className="btn-soft" data-tour="export" onClick={() => setExportOpen(true)}>{t('appbar.export')}</button>
					<button style={btn} title={t('appbar.settingsTitle')} onClick={() => setSettingsOpen(true)}>⚙</button>
					<button style={btn} title={theme === 'dark' ? t('appbar.toLight') : t('appbar.toDark')} onClick={toggleTheme}>{theme === 'dark' ? '☀' : '☾'}</button>
					<button style={btn} title={t('appbar.resetViewTitle')} onClick={() => setView({ x: 0, y: 0, scale: 1 })}>{t('appbar.resetView')}</button>
					<button className="btn-danger" title={t('appbar.clearTitle')} onClick={() => { if (window.confirm(t('confirm.clearRoom'))) clearAll() }}>{t('appbar.clear')}</button>
					<button style={btn} title={t('appbar.examplesTitle')} onClick={() => openExamples()}>{t('appbar.examples')}</button>
					<button style={btn} title={t('appbar.helpTitle')} data-tour="help" onClick={() => setGuide(true)}>?</button>
				</div>
			)}

			{/* contextual color + delete popover for a selected sticky */}
			{!READ_ONLY &&
				selectedId &&
				byId(selectedId) &&
				(() => {
					const s = byId(selectedId)!
					// keep the popover fully on-screen (esp. mobile): clamp by its own size
					const left = Math.max(8, Math.min(view.x + s.x * view.scale, size.w - 268))
					const top = Math.max(8, Math.min(view.y + s.y * view.scale - 48, size.h - 280))
					return (
						<div className="glass float-in" style={{ position: 'fixed', left, top, zIndex: 1500, display: 'flex', flexDirection: 'column', gap: 7, alignItems: 'stretch', padding: 10, width: 'min(260px, calc(100vw - 16px))', boxSizing: 'border-box' }}>
								<div style={{ display: 'flex', gap: 9, alignItems: 'center', justifyContent: 'center' }}>
									{KIND_ORDER.map((c) => (
										<button key={c} title={kindLabel(c)} onClick={() => patchShape(selectedId, { color: c })} style={{ width: 26, height: 26, padding: 0, borderRadius: '50%', background: COLORS[c], border: s.color === c ? '2px solid var(--ink)' : '2px solid var(--surface)', boxShadow: '0 1px 3px rgba(28,26,23,0.25)' }} />
									))}
								</div>
								<input placeholder={t('popover.ownerPlaceholder')} value={s.owner || ''} onChange={(e) => patchShape(selectedId, { owner: e.target.value.slice(0, 12) })} style={{ width: '100%', fontSize: 12, padding: '7px 9px', boxSizing: 'border-box' }} />
								<input placeholder={t('popover.tagsPlaceholder')} value={(s.tags || []).join(' ')} onChange={(e) => patchShape(selectedId, { tags: e.target.value.split(/[\s,]+/).filter(Boolean).slice(0, 3) })} style={{ width: '100%', fontSize: 12, padding: '7px 9px', boxSizing: 'border-box' }} />
								<div style={{ display: 'flex', gap: 8 }}>
									<button title={t('popover.dictateTitle')} className={`btn-soft${cardRecId === selectedId ? ' live' : ''}`} style={{ flex: 1, ...(cardRecId === selectedId ? { background: 'var(--live)', color: '#fff', borderColor: 'var(--live)' } : {}) }} onClick={() => dictateCard(selectedId)}>{cardRecId === selectedId ? t('popover.dictateStop') : t('popover.dictate')}</button>
									<button title={t('popover.deleteTitle')} className="btn-danger" style={{ flex: 1 }} onClick={() => { deleteSticky(selectedId); setSelectedId(null) }}>{t('popover.delete')}</button>
								</div>
							</div>
					)
				})()}

			{/* filter bar — only show cards of a tag/person */}
			{filter && (
				<div
					className="glass float-in"
					style={{ position: 'fixed', bottom: 40, left: 0, right: 0, marginInline: 'auto', width: 'fit-content', zIndex: 1400, display: 'flex', gap: 8, alignItems: 'center', padding: '6px 12px', fontSize: 13 }}
				>
					<span>{t('filter.showing', { value: filter.type === 'tag' ? '#' + filter.value : filter.value })}</span>
					<button style={{ padding: '3px 9px' }} onClick={() => setFilter(null)}>
						{t('filter.showAll')}
					</button>
				</div>
			)}

			{/* board-type picker */}
			{typePickerOpen && (
				<div
					onClick={() => setTypePickerOpen(false)}
					style={{ position: 'fixed', inset: 0, zIndex: 3500, background: 'var(--scrim)', backdropFilter: 'blur(3px)', display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 16 }}
				>
					<div className="glass modal-in" onClick={(e) => e.stopPropagation()} style={{ background: 'var(--surface)', width: 'min(420px, 92vw)', maxHeight: '88vh', overflowY: 'auto', padding: 22, borderRadius: 18 }}>
						<div style={{ fontWeight: 700, fontSize: 16 }}>{t('typePicker.title')}</div>
						<div style={{ fontSize: 12, color: 'var(--ink-soft)', margin: '4px 0 12px' }}>
							{t('typePicker.subtitle')}
						</div>
						<input
							value={newFrameTitle}
							onChange={(e) => setNewFrameTitle(e.target.value.slice(0, 40))}
							placeholder={t('typePicker.titlePlaceholder')}
							style={{ width: '100%', fontSize: 13, padding: '7px 9px', border: '1px solid var(--line)', borderRadius: 8, marginBottom: 12, boxSizing: 'border-box' }}
						/>
						{wbTypes.map((ty) => (
							<button
								key={ty.key}
								onClick={() => {
									addFrame(ty.key, newFrameTitle)
									setNewFrameTitle('')
									setTypePickerOpen(false)
								}}
								style={{ display: 'block', width: '100%', textAlign: 'left', marginBottom: 8, padding: '10px 12px', background: 'var(--surface-soft)', borderColor: 'var(--line)' }}
							>
								<div style={{ fontWeight: 600, fontSize: 14, color: 'var(--ink)' }}>{ty.label}</div>
								<div style={{ fontSize: 12, color: 'var(--ink-soft)' }}>{ty.blurb}</div>
							</button>
						))}
					</div>
				</div>
			)}

			{/* settings dialog */}
			{settingsOpen && (
				<div
					onClick={() => setSettingsOpen(false)}
					style={{ position: 'fixed', inset: 0, zIndex: 3600, background: 'var(--scrim)', backdropFilter: 'blur(3px)', display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 16 }}
				>
					<div className="glass modal-in" onClick={(e) => e.stopPropagation()} style={{ background: 'var(--surface)', width: 'min(440px, 92vw)', maxHeight: '88vh', overflowY: 'auto', padding: 22, borderRadius: 18 }}>
						<div style={{ fontWeight: 700, fontSize: 16 }}>{t('settings.title')}</div>
						<div style={{ fontSize: 12, color: 'var(--ink-soft)', margin: '4px 0 16px' }}>{t('settings.subtitle')}</div>

						{settings.adminLocked && (
							<div style={{ fontSize: 12, color: 'var(--accent)', background: 'var(--accent-soft)', border: '1px solid var(--accent)', borderRadius: 10, padding: '8px 10px', marginBottom: 14, lineHeight: 1.6 }}>
								{t('settings.adminLocked')}
							</div>
						)}

						{(() => {
							const ON = { background: 'var(--accent-soft)', borderColor: 'var(--accent)', color: 'var(--accent)' }
							return (
								<>
									<div style={{ fontWeight: 600, fontSize: 14, marginBottom: 6 }}>{t('settings.language')}</div>
									<div style={{ display: 'flex', gap: 8, marginBottom: 6 }}>
										{(
											[
												['zh-TW', '繁體中文'],
												['en', 'English'],
											] as ['zh-TW' | 'en', string][]
										).map(([code, label]) => (
											<button key={code} onClick={() => setLang(code)} style={{ flex: 1, ...(uiLang === code ? ON : {}) }}>
												{label}
											</button>
										))}
									</div>
									<div style={{ fontSize: 12, color: 'var(--ink-soft)', marginBottom: 18 }}>{t('settings.languageNote')}</div>

									<div style={{ fontWeight: 600, fontSize: 14, marginBottom: 6 }}>{t('settings.mode')}</div>
									<div style={{ display: 'flex', gap: 8, marginBottom: 6 }}>
										<button disabled={!caps.moriEar} onClick={() => saveSettings({ mode: 'mori' })} style={{ flex: 1, ...(settings.mode === 'mori' ? ON : {}) }}>
											{t('settings.modeMori')}{!caps.moriEar ? t('settings.modeMoriMissing') : ''}
										</button>
										<button onClick={() => saveSettings({ mode: 'custom' })} style={{ flex: 1, ...(settings.mode === 'custom' ? ON : {}) }}>
											{t('settings.modeCustom')}
										</button>
									</div>
									<div style={{ fontSize: 12, color: 'var(--ink-soft)', marginBottom: settings.mode === 'custom' ? 10 : 18 }}>
										{settings.mode === 'mori' ? t('settings.modeMoriDesc') : t('settings.modeCustomDesc')}
									</div>

									{settings.mode === 'custom' && (
										<div style={{ border: '1px solid var(--line)', borderRadius: 12, padding: 12, marginBottom: 18 }}>
											<div style={{ marginBottom: 12 }}>
												<div style={{ fontWeight: 600, fontSize: 13, marginBottom: 4 }}>Groq API Key {caps.groqKey ? t('settings.groqKeyOn') : groqKeyInput.trim() ? t('settings.groqKeyOnBrowser') : t('settings.groqKeyNeeded')}</div>
												<div className="muted" style={{ fontSize: 11, marginBottom: 6, lineHeight: 1.6 }}>
													{t('settings.groqKeyDesc')}
													{settings.adminLocked ? t('settings.groqKeyLockedNote') : t('settings.groqKeyLocalNote')}
												</div>
												<input type="password" value={groqKeyInput} placeholder="gsk_..." onChange={(e) => setGroqKeyInput(e.target.value)} onBlur={() => saveGroqKey(groqKeyInput.trim())} style={{ width: '100%', fontSize: 12 }} />
											</div>
											<div style={{ fontWeight: 600, fontSize: 13, marginBottom: 6 }}>{t('settings.stt')}</div>
											<div style={{ display: 'flex', gap: 8, marginBottom: 4 }}>
												<button disabled={!caps.groqKey} onClick={() => saveSettings({ sttSource: 'cloud' })} style={{ flex: 1, ...(settings.sttSource === 'cloud' ? ON : {}) }}>
													{t('settings.sttCloud')}{!caps.groqKey ? t('settings.sttCloudNoKey') : ''}
												</button>
												<button onClick={() => saveSettings({ sttSource: 'local' })} style={{ flex: 1, ...(settings.sttSource === 'local' ? ON : {}) }}>
													{t('settings.sttLocal')}
												</button>
											</div>
											<div style={{ fontSize: 11.5, color: 'var(--ink-soft)', marginBottom: 8, lineHeight: 1.6 }}>
												{settings.sttSource === 'local'
													? caps.whisperServer
														? t('settings.sttLocalDesc')
														: t('settings.sttLocalMissing')
													: t('settings.sttCloudDesc')}
											</div>
											{settings.sttSource === 'local' && (
												<input
													value={settings.whisperUrl}
													onChange={(e) => setSettings((s) => ({ ...s, whisperUrl: e.target.value }))}
													onBlur={(e) => saveSettings({ whisperUrl: e.target.value })}
													placeholder={t('settings.whisperUrlPlaceholder')}
													style={{ width: '100%', fontSize: 12, padding: '6px 8px', border: '1px solid var(--line)', borderRadius: 8, boxSizing: 'border-box', marginBottom: 12 }}
												/>
											)}
											<div style={{ fontWeight: 600, fontSize: 13, marginBottom: 6 }}>{t('settings.llm')}</div>
											<div style={{ display: 'flex', gap: 8 }}>
												<button disabled={!caps.groqKey} onClick={() => saveSettings({ localOnly: false })} style={{ flex: 1, ...(!settings.localOnly ? ON : {}) }}>
													{t('settings.llmCloud')}
												</button>
												<button onClick={() => saveSettings({ localOnly: true })} style={{ flex: 1, ...(settings.localOnly ? ON : {}) }}>
													{t('settings.llmLocal')}
												</button>
											</div>
										</div>
									)}
								</>
							)
						})()}

						<div style={{ fontWeight: 600, fontSize: 14, marginBottom: 6 }}>{t('settings.earSection')}</div>
						<div style={{ fontSize: 12, color: 'var(--ink-soft)', lineHeight: 1.6, marginBottom: 8 }}>
							<Trans i18nKey="settings.earDesc" />
						</div>
						<div style={{ fontSize: 12.5, background: 'rgba(28,26,23,0.04)', border: '1px solid var(--line)', borderRadius: 10, padding: '10px 12px', lineHeight: 1.8, marginBottom: 18 }}>
							<div>· <b>{t('settings.earSttSource')}</b> <code>stt_provider</code> = <b>{cfgInfo.sttProvider || '?'}</b>{t('settings.earSttSourceNote')}</div>
							<div>· <b>{t('settings.earCloudModel')}</b> <code>providers.groq.stt_model</code> = {cfgInfo.sttGroqModel || '?'}</div>
							<div>· <b>{t('settings.earLocalModel')}</b> <code>whisper-local.model_path</code>:</div>
							<div style={{ wordBreak: 'break-all', paddingLeft: 14, color: 'var(--ink)' }}>{cfgInfo.sttLocalModel || '?'}</div>
							<div style={{ color: 'var(--ink-soft)', paddingLeft: 14 }}>{t('settings.earModelNote')}</div>
							<hr style={{ border: 0, borderTop: '1px solid var(--line)', margin: '8px 0' }} />
							<div>· <b>{t('settings.earLlmModel')}</b> {t('settings.earLlmCloud')} <code>providers.groq.model</code> = {cfgInfo.llmGroqModel || '?'}</div>
							<div>· {t('settings.earLlmLocal')} <code>providers.ollama.model</code> = {cfgInfo.llmOllamaModel || '?'}</div>
						</div>

						<div style={{ fontWeight: 600, fontSize: 14, marginBottom: 6 }}>{t('settings.spacing')}</div>
						<div style={{ display: 'flex', gap: 8, marginBottom: 18 }}>
							{(
								[
									[t('settings.spacingTight'), 0.7],
									[t('settings.spacingNormal'), 1],
									[t('settings.spacingLoose'), 1.4],
								] as [string, number][]
							).map(([label, v]) => (
								<button
									key={label}
									onClick={() => saveSettings({ spacing: v })}
									style={{ flex: 1, ...(Math.abs(settings.spacing - v) < 0.05 ? { background: 'var(--accent-soft)', borderColor: 'var(--accent)', color: 'var(--accent)' } : {}) }}
								>
									{label}
								</button>
							))}
						</div>

						<label style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 18, cursor: 'pointer', fontSize: 13 }}>
							<input type="checkbox" checked={settings.autoTidy} onChange={(e) => saveSettings({ autoTidy: e.target.checked })} />
							{t('settings.autoTidy')}
						</label>

						<div style={{ borderTop: '1px solid var(--line)', margin: '14px 0 10px', paddingTop: 12 }}>
							<div style={{ fontWeight: 600, fontSize: 14, marginBottom: 4 }}>{t('settings.byo')}</div>
							<div className="muted" style={{ fontSize: 12, marginBottom: 8 }}>{t('settings.byoDesc')}</div>
							<input value={byo.base} onChange={(e) => saveByo({ base: e.target.value })} placeholder={t('settings.byoBasePlaceholder')} style={{ width: '100%', fontSize: 12, marginBottom: 6 }} />
							<input value={byo.key} onChange={(e) => saveByo({ key: e.target.value })} type="password" placeholder={t('settings.byoKeyPlaceholder')} style={{ width: '100%', fontSize: 12, marginBottom: 6 }} />
							<input value={byo.model} onChange={(e) => saveByo({ model: e.target.value })} placeholder={t('settings.byoModelPlaceholder')} style={{ width: '100%', fontSize: 12 }} />
							<div className="muted" style={{ fontSize: 11, marginTop: 6 }}>{byo.base.trim() && byo.key.trim() && byo.model.trim() ? t('settings.byoOn') : t('settings.byoOff')}</div>
						</div>

						<button style={{ width: '100%' }} onClick={() => setSettingsOpen(false)}>
							{t('settings.close')}
						</button>
					</div>
				</div>
			)}

			{/* export / output dialog (centered, clearly visible) */}
			{exportOpen && (
				<div
					onClick={() => setExportOpen(false)}
					style={{ position: 'fixed', inset: 0, zIndex: 3600, background: 'var(--scrim)', backdropFilter: 'blur(3px)', display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 16 }}
				>
					<div className="glass modal-in" onClick={(e) => e.stopPropagation()} style={{ background: 'var(--surface)', width: 'min(420px, 92vw)', padding: 22, borderRadius: 18 }}>
						<div style={{ fontWeight: 700, fontSize: 16 }}>{t('export.title')}</div>
						<div style={{ fontSize: 12, color: 'var(--ink-soft)', margin: '4px 0 16px' }}>{t('export.subtitle')}</div>
						<div style={{ border: '1px solid var(--line)', borderRadius: 12, padding: '11px 12px', marginBottom: 12, background: 'rgba(124,58,237,0.06)' }}>
							<div style={{ fontWeight: 600, fontSize: 14 }}>{t('export.boardFile')}</div>
							<div style={{ fontSize: 12, color: 'var(--ink-soft)', marginBottom: 8 }}>{t('export.boardFileDesc')}</div>
							<div style={{ display: 'flex', gap: 8 }}>
								<button className="btn-primary" style={{ flex: 1 }}
									onClick={() => { exportBoard(); setExportOpen(false) }}>{t('export.downloadBoard')}</button>
								<button style={{ flex: 1 }} onClick={() => pickAndImportBoard()}>{t('export.importRestore')}</button>
							</div>
						</div>
						<button
							style={{ display: 'block', width: '100%', textAlign: 'left', marginBottom: 8, padding: '11px 12px' }}
							onClick={() => {
								// window.open 帶不了 header,摘要的語言用 ?lang= 傳給 server
								window.open(`${SYNC_HTTP}/api/summary/${encodeURIComponent(room)}?lang=${apiLang()}`, '_blank')
								setExportOpen(false)
							}}
						>
							<div style={{ fontWeight: 600, fontSize: 14 }}>{t('export.summary')}</div>
							<div style={{ fontSize: 12, color: 'var(--ink-soft)' }}>{t('export.summaryDesc')}</div>
						</button>
						<button
							style={{ display: 'block', width: '100%', textAlign: 'left', marginBottom: 8, padding: '11px 12px' }}
							onClick={() => { exportHtml(); setExportOpen(false) }}
						>
							<div style={{ fontWeight: 600, fontSize: 14 }}>{t('export.html')}</div>
							<div style={{ fontSize: 12, color: 'var(--ink-soft)' }}>{t('export.htmlDesc')}</div>
						</button>
						<button
							style={{ display: 'block', width: '100%', textAlign: 'left', marginBottom: 8, padding: '11px 12px' }}
							onClick={() => {
								exportMd()
								setExportOpen(false)
							}}
						>
							<div style={{ fontWeight: 600, fontSize: 14 }}>{t('export.md')}</div>
							<div style={{ fontSize: 12, color: 'var(--ink-soft)' }}>{t('export.mdDesc')}</div>
						</button>
						<div style={{ border: '1px solid var(--line)', borderRadius: 12, padding: '11px 12px', marginBottom: 12 }}>
							<div style={{ fontWeight: 600, fontSize: 14 }}>{t('export.png')}</div>
							<div style={{ fontSize: 12, color: 'var(--ink-soft)', marginTop: 2 }}>{t('export.pngDesc')}</div>
							<div style={{ display: 'flex', gap: 16, margin: '8px 0', fontSize: 13 }}>
								<label style={{ display: 'inline-flex', alignItems: 'center', gap: 5, cursor: 'pointer' }}>
									<input type="radio" checked={!pngTransparent} onChange={() => setPngTransparent(false)} /> {t('export.pngPaper')}
								</label>
								<label style={{ display: 'inline-flex', alignItems: 'center', gap: 5, cursor: 'pointer' }}>
									<input type="radio" checked={pngTransparent} onChange={() => setPngTransparent(true)} /> {t('export.pngTransparent')}
								</label>
							</div>
							<div style={{ display: 'flex', gap: 8 }}>
								<button
									className="btn-primary" style={{ flex: 1 }}
									onClick={() => {
										void exportPng(pngTransparent)
										setExportOpen(false)
									}}
								>
									{t('export.downloadPng')}
								</button>
								<button
									style={{ flex: 1 }}
									title={t('export.copyPngTitle')}
									onClick={() => {
										void copyPngToClipboard()
										setExportOpen(false)
									}}
								>
									{t('export.copyPng')}
								</button>
							</div>
						</div>
						<button style={{ width: '100%' }} onClick={() => setExportOpen(false)}>
							{t('export.close')}
						</button>
					</div>
				</div>
			)}

			{/* transient STT caption (UX feedback) */}
			{subtitle && (
				<div
					className="modal-in"
					style={{
						position: 'fixed',
						bottom: mobile ? 92 : 74,
						left: 0,
						right: 0,
						marginInline: 'auto',
						width: 'fit-content',
						zIndex: 1600,
						maxWidth: '80vw',
						padding: '8px 16px',
						borderRadius: 12,
						background: 'rgba(28,26,23,0.82)',
						color: '#fff',
						fontSize: 15,
						textAlign: 'center',
						pointerEvents: 'none',
					}}
				>
					{subtitle}
				</div>
			)}

			{/* hint (desktop only) */}
			{!mobile && (
				<div style={hint}>
					{t('hint.desktop')}
				</div>
			)}

							{/* demo / sponsor banner (only when the host sets SPONSOR_URL / DEMO_NOTICE env) */}
				{iosInstallHint && (
					<div className="scrim" style={{ zIndex: 4000 }} onClick={() => setIosInstallHint(false)}>
						<div className="dialog-card modal-in" onClick={(e) => e.stopPropagation()} style={{ width: 'min(360px, 92vw)', textAlign: 'center' }}>
							<div style={{ fontWeight: 700, fontSize: 17, marginBottom: 10 }}>{t('install.iosTitle')}</div>
							<div className="muted" style={{ fontSize: 13.5, lineHeight: 1.7, marginBottom: 16 }}>
								<Trans i18nKey="install.iosBody" />
							</div>
							<button className="btn-accent" style={{ width: '100%' }} onClick={() => setIosInstallHint(false)}>{t('install.gotIt')}</button>
						</div>
					</div>
				)}
				{(sponsor.notice || sponsor.url) && !sponsorHidden && (
					<div className="glass float-in" style={{ position: 'fixed', bottom: 'calc(14px + env(safe-area-inset-bottom, 0px))', right: 'calc(14px + env(safe-area-inset-right, 0px))', zIndex: 1300, display: 'flex', alignItems: 'center', gap: 10, padding: '8px 12px', maxWidth: 'min(92vw, 430px)', fontSize: 12.5 }}>
						{sponsor.notice && <span className="muted" style={{ lineHeight: 1.35 }}>{sponsor.notice}</span>}
						{sponsor.url && (
							<a href={sponsor.url} target="_blank" rel="noreferrer" style={{ flex: '0 0 auto' }}>
								<button className="btn-accent" style={{ padding: '5px 11px' }}>{sponsor.label || t('sponsor.label')}</button>
							</a>
						)}
						<button title={t('sponsor.closeTitle')} style={{ flex: '0 0 auto', padding: '3px 8px' }} onClick={() => setSponsorHidden(true)}>✕</button>
					</div>
				)}

				{/* agent / voice panel (collapsible; record stays visible) */}
			{!READ_ONLY && (<div className="glass float-in" data-tour="paste" style={{ ...panel, width: mobile ? 'min(86vw, 320px)' : 320, left: mobile ? 8 : 14 }}>
				<div
					onClick={() => setPanelOpen((o) => !o)}
					style={{ fontWeight: 600, cursor: 'pointer', userSelect: 'none' }}
				>
					{panelOpen ? '▾' : '▸'} {t('panel.title')}
				</div>
				{/* voice = the main way to get content onto the board */}
				<button
					className={`btn-rec${meeting ? ' live' : ''}`}
					data-tour="record"
						style={{ width: '100%', marginTop: 8, fontSize: 15, padding: '13px', fontWeight: 600, display: 'flex', alignItems: 'center', justifyContent: 'center' }}
					title={t('panel.recordTitle')}
					onClick={() => (meeting ? stopMeeting() : startMeeting())}
				>
					<span className="rec-dot" />{meeting ? t('panel.recording', { count: segCount }) : t('panel.record')}
					{meeting && <VolBars level={meterRef} />}
				</button>
				{(pendingSegs > 0 || failedSegs.length > 0) && (
					<div style={{ display: 'flex', gap: 6, marginTop: 6, fontSize: 12, alignItems: 'center', flexWrap: 'wrap' }}>
						{pendingSegs > 0 && (
							<span className="muted" style={{ border: '1px solid var(--line)', borderRadius: 999, padding: '2px 10px' }}>
								{t('panel.pending', { count: pendingSegs })}
							</span>
						)}
						{failedSegs.length > 0 && (
							<button
								style={{ border: '1px solid var(--live)', color: 'var(--live)', background: 'transparent', borderRadius: 999, padding: '2px 10px', fontSize: 12, cursor: 'pointer' }}
								title={t('panel.failedRetryTitle')}
								onClick={() => void retryFailedSegs()}
							>
								{t('panel.failedRetry', { count: failedSegs.length })}
							</button>
						)}
					</div>
				)}
				{panelOpen && (
					<>
						<button
							style={{ ...btn, width: '100%', marginTop: 6, fontSize: 12, ...(recording ? { background: 'var(--live)', color: '#fff', borderColor: 'var(--live)' } : {}) }}
							title={t('panel.onceTitle')}
							onClick={toggleRecord}
							disabled={meeting}
						>
							{recording ? t('panel.onceStop') : t('panel.once')}
						</button>
						{/* manual transcript = secondary, hidden until you ask for it */}
						<div style={{ marginTop: 8, fontSize: 12 }}>
							<span onClick={() => setShowPaste((v) => !v)} style={{ cursor: 'pointer', color: 'var(--accent)' }}>
								{showPaste ? t('panel.hidePaste') : t('panel.showPaste')}
							</span>
						</div>
						{showPaste && (
							<>
								<textarea
									value={agentText}
									onChange={(e) => setAgentText(e.target.value)}
									placeholder={t('panel.pastePlaceholder')}
									style={{ width: '100%', height: 70, fontSize: 12, resize: 'vertical', boxSizing: 'border-box', marginTop: 6 }}
								/>
								<button title={t('panel.sendToAgentTitle')} style={{ ...btn, width: '100%', marginTop: 6 }} onClick={runAgent}>
									{t('panel.sendToAgent')}
								</button>
							</>
						)}
					</>
				)}
				{panelOpen && transcript.length > 0 && (
					<div style={{ marginTop: 10, borderTop: '1px solid var(--line)', paddingTop: 8 }}>
						<div className="muted" style={{ fontSize: 11, marginBottom: 5 }}>{t('panel.transcript', { count: transcript.length })}</div>
						<div style={{ maxHeight: 150, overflowY: 'auto', fontSize: 12, lineHeight: 1.5 }}>
							{transcript.map((e: any, i: number) => (
								<div key={i} style={{ marginBottom: 4 }}>
									<span className="muted" style={{ fontSize: 10 }}>{(e.t || '').slice(11, 16)} {e.by} </span>
									{e.text}
								</div>
							))}
							<div ref={transcriptEndRef} />
						</div>
					</div>
				)}
				{lastAiIds.length > 0 && (
					<button
						className="btn-soft"
						style={{ width: '100%', marginTop: 6, fontSize: 12 }}
						title={t('panel.undoAiTitle')}
						onClick={undoLastAi}
					>
						{t('panel.undoAi', { count: lastAiIds.length })}
					</button>
				)}
				{busy && <div style={{ marginTop: 6, fontSize: 12, color: 'var(--ink-soft)' }}>{busy}</div>}
			</div>)}
			{/* 唯讀檢視沒有開會記錄面板,busy 提示改浮在左下 */}
			{READ_ONLY && busy && (
				<div className="glass" style={{ position: 'fixed', left: 14, bottom: 14, zIndex: 1200, padding: '8px 14px', fontSize: 12.5, color: 'var(--ink-soft)' }}>{busy}</div>
			)}

			{/* share modal: QR + 房號 + join-by-code */}
			{shareOpen && (
				<div
					onClick={() => setShareOpen(false)}
					style={{
						position: 'fixed',
						inset: 0,
						zIndex: 3000,
						background: 'var(--scrim)',
						display: 'flex',
						alignItems: 'center',
						justifyContent: 'center',
						font: '14px system-ui, sans-serif',
					}}
				>
					<div
						className="modal-in" onClick={(e) => e.stopPropagation()}
						style={{
							background: 'var(--surface)',
							borderRadius: 18,
							padding: 20,
							width: 'min(340px, 92vw)',
							maxHeight: '90dvh',
							overflowY: 'auto',
							boxSizing: 'border-box',
							textAlign: 'center',
							boxShadow: '0 24px 60px -20px rgba(28,26,23,0.45)',
						}}
					>
						<div style={{ display: 'flex', gap: 6, alignItems: 'center', marginBottom: 12 }}>
							<span style={{ color: 'var(--ink-soft)', fontSize: 13, whiteSpace: 'nowrap' }}>{t('share.yourName')}</span>
							<input
								value={myName}
								onChange={(e) => setMyName(e.target.value.slice(0, 24))}
								placeholder={t('share.namePlaceholder')}
								style={{ flex: 1, font: '14px system-ui', padding: '6px 8px', border: '1px solid var(--line)', borderRadius: 6 }}
							/>
						</div>
						<div style={{ color: 'var(--ink-soft)', fontSize: 13 }}>{t('share.scanHint')}</div>
						<div className="code" style={{ fontSize: 52, color: 'var(--accent)', margin: '6px 0 16px', lineHeight: 1 }}>{room}</div>
						{qrUrl ? (
							<img src={qrUrl} alt="QR" style={{ width: '100%', maxWidth: 240, height: 'auto', border: '1px solid var(--line)', borderRadius: 8 }} />
						) : (
							<div style={{ height: 240, lineHeight: '240px', color: 'var(--ink-soft)' }}>{t('share.qrLoading')}</div>
						)}
						<div style={{ display: 'flex', gap: 6, marginTop: 12 }}>
							<input
								readOnly
								value={shareUrl}
								onFocus={(e) => e.currentTarget.select()}
								style={{ flex: 1, font: '12px system-ui', padding: '6px 8px', border: '1px solid var(--line)', borderRadius: 6 }}
							/>
							<button
								className="btn-soft"
								onClick={() => navigator.clipboard?.writeText(shareUrl)}
							>
								{t('share.copyLink')}
							</button>
						</div>
						<hr style={{ margin: '16px 0', border: 0, borderTop: '1px solid var(--line)' }} />
						<div style={{ display: 'flex', gap: 6 }}>
							<input
								value={joinCode}
								onChange={(e) => setJoinCode(e.target.value)}
								onKeyDown={(e) => e.key === 'Enter' && joinRoom()}
								placeholder={t('share.joinPlaceholder')}
								style={{ flex: 1, font: '14px system-ui', padding: '6px 8px', border: '1px solid var(--line)', borderRadius: 6, textTransform: 'uppercase' }}
							/>
							<button style={btn} onClick={joinRoom}>
								{t('share.join')}
							</button>
						</div>
						{roomList.length > 0 ? (
							<div style={{ marginTop: 14, textAlign: 'left', maxHeight: 140, overflowY: 'auto' }}>
								<div style={{ color: 'var(--ink-soft)', fontSize: 12, marginBottom: 4 }}>{t('share.activeRooms')}</div>
								{roomList.map((r) => (
									<div key={r.id} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 0' }}>
										<span style={{ flex: 1, fontWeight: r.id === room ? 700 : 400 }}>
											{r.id} <span style={{ color: 'var(--ink-soft)', fontSize: 12 }}>{t('share.roomMeta', { online: r.online, shapes: r.shapes })}</span>
										</span>
										{r.id !== room && (
											<button
												style={{ ...btn, padding: '2px 8px' }}
												onClick={() => (location.href = `${location.pathname}?room=${encodeURIComponent(r.id)}`)}
											>
												{t('share.enter')}
											</button>
										)}
									</div>
								))}
							</div>
						) : roomCount > 0 ? (
							// 此站不公開房號(房號即進房鑰匙):只顯示數量,要進別房請輸入房號
							<div style={{ marginTop: 14, color: 'var(--ink-soft)', fontSize: 12 }}>
								{t('share.roomCountOnly', { count: roomCount })}
							</div>
						) : null}
						{!READ_ONLY && (
							<div style={{ display: 'flex', gap: 6, marginTop: 10 }}>
								<button style={{ ...btn, flex: 1 }} title={t('share.copyViewLinkTitle')} onClick={copyViewLink}>
									{t('share.copyViewLink')}
								</button>
								<button
									style={{ ...btn, flex: 1, ...(roomLocked ? { borderColor: 'var(--accent)', color: 'var(--accent)' } : {}) }}
									title={roomLocked ? t('share.unlockTitle') : t('share.lockTitle')}
									onClick={toggleLock}
								>
									{roomLocked ? t('share.unlock') : t('share.lock')}
								</button>
							</div>
						)}
						{roomLocked && (
							<div className="muted" style={{ marginTop: 6, fontSize: 12 }}>{t('share.lockedNote')}</div>
						)}
						<div style={{ display: 'flex', gap: 6, marginTop: 10 }}>
							{!READ_ONLY && (
								<button style={{ ...btn, flex: 1, color: 'var(--live)' }} onClick={endThisRoom}>
									{t('share.endRoom')}
								</button>
							)}
							<button style={{ ...btn, flex: 1 }} onClick={() => setShareOpen(false)}>
								{t('share.close')}
							</button>
						</div>
					</div>
				</div>
			)}
		</div>
	)
}

// these get className="glass" for the frosted look; consts hold position/layout only
// 置中不能用 left:50% + translateX(-50%):float-in/modal-in 動畫的 transform 會把它蓋掉
// (fill-mode both + to{transform:none}),動畫結束就歪掉。改用 inset+margin 置中。
const bar: React.CSSProperties = {
	position: 'fixed',
	top: 14,
	left: 0,
	right: 0,
	marginInline: 'auto',
	width: 'fit-content',
	zIndex: 1000,
	display: 'flex',
	flexWrap: 'wrap',
	justifyContent: 'center',
	maxWidth: '94vw',
	gap: 6,
	alignItems: 'center',
	padding: '7px 12px',
	fontSize: 13,
}
const appbar: React.CSSProperties = {
	position: 'fixed',
	top: 14,
	right: 14,
	zIndex: 1000,
	display: 'flex',
	flexWrap: 'wrap',
	justifyContent: 'flex-end',
	gap: 6,
	alignItems: 'center',
	padding: '7px 10px',
	fontSize: 13,
	maxWidth: '46vw',
}
const hint: React.CSSProperties = {
	position: 'fixed',
	bottom: 10,
	left: '50%',
	transform: 'translateX(-50%)',
	zIndex: 1000,
	color: 'var(--ink-soft)',
	fontSize: 12,
}
const panel: React.CSSProperties = {
	position: 'fixed',
	left: 14,
	bottom: 'calc(38px + env(safe-area-inset-bottom, 0px))',
	zIndex: 1000,
	padding: 12,
	fontSize: 13,
}
