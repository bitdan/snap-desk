import { appearance } from './appearance'
import messages from './messages.en.json'
import nativeMessages from './messages.native.en.json'
import errorSlots from './message-error-slots.json'

/** 中文消息键只用于界面；业务数据不送入词典。前后端共享原生消息表。 */
export const english: Record<string, string> = { ...nativeMessages, ...messages }

// 模板消息兼容原生/纯逻辑层返回的中文状态；只在明确的界面消息位置调用。
// 参数当作原始内容插入，不递归翻译，避免改写文件路径、识别文字和服务错误详情。
const templates = Object.entries(english).filter(([key]) => /\{\d+\}/.test(key)).map(([key, value]) => {
  const names: string[] = []
  const escaped = key.split(/(\{\d+\})/).map(part => {
    if (/^\{\d+\}$/.test(part)) { names.push(part.slice(1, -1)); return '([\\s\\S]*?)' }
    return part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  }).join('')
  return { pattern: new RegExp(`^${escaped}$`), names, value, errors: (errorSlots as Record<string, number[]>)[key] ?? [] }
})

/** 中文是稳定消息键；切换语言只影响显示，不改变内部状态和业务数据。 */
function translate(message: string | null | undefined, values?: Record<string, string | number | null | undefined>, errorDepth?: number): string {
  const source = message ?? ''
  let translated = source
  if (appearance.value.language === 'en') {
    translated = Object.hasOwn(english, source) ? english[source] : source
    if (translated === source && !Object.hasOwn(english, source) && source.length <= 8192) {
      for (const template of templates) {
        const match = template.pattern.exec(source)
        if (!match) continue
        const captures = Object.fromEntries(template.names.map((name, index) => {
          const value = match[index + 1]
          // 只有明确标记的应用错误字段才递归翻译；路径、OCR 内容等参数始终原样插入。
          return [name, errorDepth !== undefined && errorDepth < 4 && template.errors.includes(Number(name))
            ? translateError(value, errorDepth + 1) : value]
        }))
        return template.value.replace(/\{(\d+)\}/g, (token, name: string) => captures[name] ?? token)
      }
    }
  }
  return values ? translated.replace(/\{(\d+)\}/g, (token, name: string) => Object.hasOwn(values, name) ? String(values[name] ?? '') : token) : translated
}

/** 已知文案和显式参数只用于显示；业务状态仍保留中文消息键。 */
export function t(message: string | null | undefined, values?: Record<string, string | number | null | undefined>): string {
  return translate(message, values)
}

function translateError(source: string, depth: number): string {
  const prefix = /^(Error: )?([a-z][a-z0-9_]*[：:]\s*)?/.exec(source)![0]
  return prefix + translate(source.slice(prefix.length), undefined, depth)
}

/** IPC 包装保留命令名，已知的内层应用错误翻译；未知系统/驱动详情保持原文。 */
export function te(message: string | null | undefined): string {
  return translateError(message ?? '', 0)
}

/** 时间跟随应用语言；原始时间戳始终保留在数据层。 */
export function localizedDate(milliseconds: number): string {
  return new Date(milliseconds).toLocaleString(appearance.value.language === 'en' ? 'en-US' : 'zh-CN')
}
