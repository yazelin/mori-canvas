// i18n bootstrap — zh-TW is the source of truth, en mirrors its key set.
// Detection: localStorage 'wb-lang' > navigator.language (zh* -> zh-TW, else en).
// Proper nouns (Mori Canvas, Groq, Ollama, whisper…) stay out of the locale files.
import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import zhTW from './locales/zh-TW.json'
import en from './locales/en.json'

export function detectLang(): 'zh-TW' | 'en' {
	try {
		const stored = localStorage.getItem('wb-lang')
		if (stored === 'zh-TW' || stored === 'en') return stored
	} catch {}
	return (navigator.language || '').toLowerCase().startsWith('zh') ? 'zh-TW' : 'en'
}

i18n.use(initReactI18next).init({
	resources: { 'zh-TW': { translation: zhTW }, en: { translation: en } },
	lng: detectLang(),
	fallbackLng: 'zh-TW',
	interpolation: { escapeValue: false }, // React escapes already
	returnEmptyString: true,
})

// keep <html lang> + the visible tab title in sync (a11y / font selection / search engines)
// (index.html 的 SEO/OG meta 是靜態 zh 預設;站點級英文頁屬 Pages 改版範圍)
const syncHtmlLang = (lng: string) => {
	document.documentElement.lang = lng === 'en' ? 'en' : 'zh-TW'
	document.title = i18n.t('app.title')
}
syncHtmlLang(i18n.language)
i18n.on('languageChanged', syncHtmlLang)

/** language tag sent to the server with every AI request (X-Lang header / ?lang=) */
export const apiLang = (): 'zh-TW' | 'en' => (i18n.language === 'en' ? 'en' : 'zh-TW')

/** user picked a language in settings: switch live + remember */
export function setLang(lng: 'zh-TW' | 'en') {
	void i18n.changeLanguage(lng)
	try {
		localStorage.setItem('wb-lang', lng)
	} catch {}
}

export default i18n
