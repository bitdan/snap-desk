/** 轻量中英主体语言判断；仅选择翻译方向，原文仍完整交给翻译服务。 */
export type TextLanguage = 'zh-CN' | 'en' | 'mixed' | 'unknown'

export function detectTextLanguage(text: string): TextLanguage {
  const prose = text.replace(/```[\s\S]*?(?:```|$)/g, ' ')
    .replace(/`[^`\n]+`/g, ' ')
    .replace(/\b(?:https?:\/\/|www\.)[^\s<>()]+/gi, ' ')
    .replace(/\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi, ' ')
    .replace(/\b[A-Z]:[\\/][^\s]+/gi, ' ')
  // 两个汉字近似一个中文词，避免长英文术语按字母数压过中文句子的主体。
  const chinese = (prose.match(/\p{Script=Han}/gu)?.length ?? 0) / 2
  const english = prose.match(/\p{Script=Latin}+(?:['’-]\p{Script=Latin}+)*/gu)?.length ?? 0
  if (!chinese && !english) return 'unknown'
  const ratio = chinese / (chinese + english)
  if (ratio >= 0.6) return 'zh-CN'
  if (ratio <= 0.4) return 'en'
  return 'mixed'
}

/** 混合比例接近或没有语言内容时沿用默认目标，不改写用户的持久设置。 */
export function automaticTarget(language: TextLanguage, defaultTarget: string): string {
  return language === 'zh-CN' ? 'en' : language === 'en' ? 'zh-CN' : defaultTarget
}
