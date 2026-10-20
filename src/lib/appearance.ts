import { ref } from 'vue'

export type Theme = 'system' | 'light' | 'dark'
export type Language = 'zh-CN' | 'en'
export interface Appearance { theme: Theme; language: Language }
export const appearance = ref<Appearance>({ theme: 'system', language: 'zh-CN' })

/** 配置文件是事实源；本地缓存仅用于首帧，避免深色启动时闪白。 */
export function applyAppearance(value: Appearance) {
  // 无关设置刷新时不替换引用，避免覆盖外观页尚未保存的草稿。
  if (value.theme !== appearance.value.theme || value.language !== appearance.value.language)
    appearance.value = { ...value }
  renderAppearance()
  try { localStorage.setItem('snap-desk-appearance', JSON.stringify(value)) } catch { /* 缓存失败不影响配置保存。 */ }
}
export function parseAppearance(value: unknown): Appearance | null {
  if (!value || typeof value !== 'object') return null
  const v = value as Partial<Appearance>
  return ['system', 'light', 'dark'].includes(v.theme ?? '') && ['zh-CN', 'en'].includes(v.language ?? '')
    ? { theme: v.theme!, language: v.language! } : null
}
function renderAppearance() {
  const { theme, language } = appearance.value
  document.documentElement.dataset.theme = theme === 'system'
    ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : theme
  document.documentElement.lang = language
}
/** 每个 WebView 注册一次。storage 事件同步已经打开的辅助窗口。 */
export function initializeAppearance() {
  try {
    const cached = parseAppearance(JSON.parse(localStorage.getItem('snap-desk-appearance') || 'null'))
    if (cached) appearance.value = cached
  } catch { /* 损坏缓存回退默认值，随后由后端覆盖。 */ }
  renderAppearance()
  const media = matchMedia('(prefers-color-scheme: dark)')
  media.addEventListener('change', renderAppearance)
  window.addEventListener('storage', event => {
    if (event.key !== 'snap-desk-appearance') return
    try { const value = parseAppearance(JSON.parse(event.newValue || 'null')); if (value) { appearance.value = value; renderAppearance() } } catch { /* 忽略损坏的缓存。 */ }
  })
}
