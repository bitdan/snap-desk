/** 提示文案拆成说明与快捷键两部分，快捷键单独渲染成按键样式。 */
export interface TooltipText { label: string; keys: string[] }

const keyToken = /^(?:Ctrl|⌘|Alt|Shift|Esc|Enter|Delete|Backspace|Tab|Space|Home|End|F\d{1,2}|[A-Z0-9,.\-=+←→↑↓]|Ctrl\/⌘)(?:\+\S+)*$/
const cjk = /[　-ヿ㐀-鿿＀-￯]/

function keysOf(value: string): string[] | null {
  const text = value.trim()
  if (!text || cjk.test(text) || text.length > 24) return null
  // 多个快捷键用顿号或斜杠后的空格分隔时不拆，保留原文更不易误读。
  if (/\s/.test(text) || !keyToken.test(text)) return null
  return text === '+' ? ['+'] : text.split(/(?<=.)\+(?=.)/)
}

export function splitTooltip(text: string): TooltipText {
  const value = text.trim()
  // 末尾括注：收起工具（Tab）、上一张截图（,）
  const paren = /^(.*?)\s*[（(]([^（）()]+)[）)]$/.exec(value)
  if (paren?.[1]) {
    const keys = keysOf(paren[2]!)
    if (keys) return { label: paren[1], keys }
  }
  // 末尾空格后的快捷键：复制 Ctrl/⌘+C、关闭 Esc
  const tail = /^(.*\S)\s+(\S+)$/.exec(value)
  if (tail) {
    const keys = keysOf(tail[2]!)
    if (keys) return { label: tail[1]!, keys }
  }
  return { label: value, keys: [] }
}
