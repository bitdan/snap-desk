/** 输入框与导出共用换行规则；尺寸使用原图像素，避免预览能容纳而提交时溢出。 */
export function annotationTextLayout(text: string, fontSize: number, availableWidth: number, measure: (text: string) => number) {
  const maxWidth = Math.max(1, Math.min(2048, Math.floor(availableWidth))), lines: string[] = []
  for (const paragraph of text.split('\n')) {
    let line = ''
    for (const char of paragraph) {
      if (line && measure(line + char) > maxWidth - 4) { lines.push(line); line = '' }
      line += char
    }
    lines.push(line)
  }
  return { lines, width: Math.min(maxWidth, Math.max(1, Math.ceil(Math.max(...lines.map(measure)) + 4))), height: Math.max(1, Math.ceil(lines.length * fontSize * 1.3 + 4)) }
}
