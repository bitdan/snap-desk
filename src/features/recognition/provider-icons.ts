import { brandIcons, serviceBrand } from '@/shared/ui/brand-icons'

/** 引擎与服务的图标：有品牌图标时返回 SVG 源码，否则返回字母块与底色。 */
export interface ProviderIcon { icon: string; mono: string; tile: string }

const monograms: Record<string, [string, string]> = {
  'ocr-space': ['OS', '#2f7d4f'], 'wechat-ocr': ['WX', '#07a35a'], offline: ['CT', '#475569'], custom: ['+', '#7c5cdb'],
}
const localBrands: Record<string, keyof typeof brandIcons> = { 'local-ppocr': 'baidu', 'windows-media': 'microsoft' }

/**
 * value 为菜单值（preset:<id>、service:<id> 或本地引擎名）；已保存的服务另传 presetId 与接口地址，
 * 以便自定义地址指向已知服务商时也显示对应图标。
 */
export function providerIcon(value: string, label: string, presetId?: string, endpoint?: string): ProviderIcon {
  const key = presetId ?? (value.startsWith('preset:') ? value.slice(7) : value)
  const brand = localBrands[key] ?? serviceBrand(key, endpoint ?? defaultEndpoints[key] ?? '')
  if (brand && brandIcons[brand]) return { icon: brandIcons[brand], mono: '', tile: '' }
  const [mono, tile] = monograms[key] ?? [label.replace(/[^\p{L}\p{N}]/gu, '').slice(0, 2).toUpperCase() || '?', '#6b7280']
  return { icon: '', mono, tile }
}

// 预设未保存时没有接口地址；用预设默认地址识别 OpenAI、本机 Ollama 等服务商。
const defaultEndpoints: Record<string, string> = {
  openai: 'https://api.openai.com/v1/chat/completions',
  local: 'http://127.0.0.1:11434/v1/chat/completions',
}
