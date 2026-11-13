/** 服务商图标（MIT，来源见 assets/brands/README.md）；以字符串内联，单色图标随主题文字色变化。 */

// 用 glob 在构建时内联全部图标源码，文件名（去掉 -color 后缀）即服务商键。
const files = import.meta.glob<string>('../assets/brands/*.svg', { query: '?raw', import: 'default', eager: true })
const icon = (name: string) => files[`../assets/brands/${name}.svg`] ?? ''
export const brandIcons = {
  baidu: icon('baidu-color'), deepl: icon('deepl-color'), deepseek: icon('deepseek-color'), groq: icon('groq'),
  microsoft: icon('microsoft-color'), ollama: icon('ollama'), openai: icon('openai'),
} as const
export type BrandIcon = keyof typeof brandIcons

/** 按预设或接口地址识别服务商；识别不出时返回 undefined，界面退回字母图标。 */
export function serviceBrand(presetId: string, endpoint = ''): BrandIcon | undefined {
  const byPreset: Record<string, BrandIcon> = { deepseek: 'deepseek', groq: 'groq', 'groq-vision': 'groq', deepl: 'deepl', 'paddle-ocr': 'baidu' }
  if (byPreset[presetId]) return byPreset[presetId]
  let host = '', port = ''
  try { const url = new URL(endpoint); host = url.hostname; port = url.port } catch { return undefined }
  if (host === 'api.openai.com' || host.endsWith('.openai.com')) return 'openai'
  if (host.endsWith('deepseek.com')) return 'deepseek'
  if (host.endsWith('groq.com')) return 'groq'
  if (host.endsWith('deepl.com')) return 'deepl'
  if (['localhost', '127.0.0.1', '[::1]'].includes(host) && port === '11434') return 'ollama'
  return undefined
}
