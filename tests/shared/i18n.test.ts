import { afterEach, describe, expect, it } from 'vitest'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { parse } from '@vue/compiler-sfc'
import { baseParse, type TemplateChildNode, type RootNode } from '@vue/compiler-dom'
import ts from 'typescript'
import { appearance } from '../../src/shared/ui/appearance'
import { english, localizedDate, t, te } from '../../src/shared/i18n/i18n'
import { annotationTools, annotationToolGroups } from '../../src/features/annotations/annotation-tools'
import { targetLanguages } from '../../src/features/recognition/ai'
import { exportFilenamePreview, shortcutActions } from '../../src/features/settings/settings'
import { encoderLabels, recordingAudioStatus, recordingLabels } from '../../src/features/recording/recording'

afterEach(() => { appearance.value.language = 'zh-CN' })
const han = (value: string) => /\p{Script=Han}/u.test(value)
function files(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const file = join(directory, entry.name)
    return entry.isDirectory() ? files(file) : [file]
  })
}

describe('application language coverage', () => {
  it('switches existing messages in both directions without rewriting parameters or unknown details', () => {
    const path = 'F:\\图片\\截图.png'
    const text = '保存图片 $& {0} <script>'
    appearance.value.language = 'en'
    expect(t('保存图片')).toBe('Save image')
    expect(t('图片 {0} × {1}', { 0: null, 1: 12 })).toBe('Image  × 12')
    expect(te('Error: start_recording：WASAPI 音频录制失败：0x123。设备已断开或失效，请重新连接设备后重新录制。'))
      .toBe('Error: start_recording：WASAPI recording failed: 0x123. Device disconnected or invalidated. Reconnect and record again.')
    expect(t('译文：{0}', { 0: text })).toBe(`Translation: ${text}`)
    expect(te(`Error: save_capture：无法打开目录：${path}`)).toBe(`Error: save_capture：Could not open folder: ${path}`)
    expect(t('平均 29.5 FPS · 近 2 秒 30.0 FPS')).toBe('Average 29.5 FPS · Last 2 seconds 30.0 FPS')
    expect(te('driver error 中文详情 0x80004005')).toBe('driver error 中文详情 0x80004005')
    for (const key of ['__proto__', 'constructor', 'toString']) expect(t(key)).toBe(key)
    appearance.value.language = 'zh-CN'
    expect(t('保存图片')).toBe('保存图片')
    expect(t('译文：{0}', { 0: text })).toBe(`译文：${text}`)
  })

  it('preserves every message placeholder and supplies English for the shipped control catalogs', () => {
    for (const [key, value] of Object.entries(english)) {
      expect(value.trim(), key).not.toBe('')
      expect([...value.matchAll(/\{\d+\}/g)].map(m => m[0]).sort(), key)
        .toEqual([...key.matchAll(/\{\d+\}/g)].map(m => m[0]).sort())
    }
    appearance.value.language = 'en'
    const labels = [...annotationTools, ...annotationToolGroups].map(item => item.name)
      .concat(targetLanguages.map(item => item.label), shortcutActions.flatMap(item => [item.label, item.description]),
        Object.values(encoderLabels), Object.values(recordingLabels))
    for (const label of labels) if (label !== '简体中文') expect(han(t(label)), label).toBe(false)
    expect(recordingAudioStatus({ systemAudio: true, microphone: true })).toBe('System audio + Microphone')
    expect(recordingAudioStatus({ systemAudio: false, microphone: false })).toBe('Silent')
    expect(exportFilenamePreview({ filenamePrefix: '图片-时间戳', includeDimensions: true, format: 'png', jpegQuality: 90, rememberDirectory: true }))
      .toBe('图片-时间戳-timestamp-1920x1080.png')
    expect(localizedDate(0)).toBe(new Date(0).toLocaleString('en-US'))
  })

  it('keeps visible Chinese literals behind translation calls in every Vue window', () => {
    const issues: string[] = []
    function expression(source: string, file: string) {
      const ast = ts.createSourceFile('expression.ts', `(${source})`, ts.ScriptTarget.Latest, true)
      function visit(node: ts.Node, translated = false) {
        const inside = translated || (ts.isCallExpression(node) && ['t', 'te'].includes(node.expression.getText(ast)))
        if (ts.isStringLiteral(node) && han(node.text)) {
          // 状态比较仍用稳定中文值；显示字面量必须经过 t/te。
          const comparison = ts.isBinaryExpression(node.parent) && ['===', '!=='].includes(node.parent.operatorToken.getText(ast))
          if (!inside && !comparison) issues.push(`${file}: ${node.text}`)
          if (inside && !Object.hasOwn(english, node.text)) issues.push(`${file}: missing ${node.text}`)
        }
        ts.forEachChild(node, child => visit(child, inside))
      }
      visit(ast)
    }
    for (const file of files('src').filter(file => file.endsWith('.vue'))) {
      const template = parse(readFileSync(file, 'utf8')).descriptor.template
      if (!template) continue
      function visit(node: RootNode | TemplateChildNode) {
        if (node.type === 2 && han(node.content)) issues.push(`${file}: ${node.content}`)
        if (node.type === 5 && node.content.type === 4) expression(node.content.content, file)
        if (node.type === 1) for (const prop of node.props) {
          if (prop.type === 6 && prop.value && ['title', 'aria-label', 'placeholder', 'alt'].includes(prop.name) && han(prop.value.content))
            issues.push(`${file}: ${prop.value.content}`)
          if (prop.type === 7 && prop.name === 'bind' && prop.exp?.type === 4 && prop.arg?.type === 4 && ['title', 'aria-label', 'placeholder', 'alt'].includes(prop.arg.content))
            expression(prop.exp.content, file)
        }
        if (node.type === 0 || node.type === 1) for (const child of node.children) visit(child)
      }
      visit(baseParse(template.content))
    }
    expect(issues).toEqual([])
  })

  it('keeps OCR, QR, translations, group names and paths outside translation calls', () => {
    for (const [file, expressions] of [
      ['src/features/recognition/components/ImageTranslation.vue', ['unit.source', 'results[unit.id]']],
      ['src/features/recognition/components/PinRecognition.vue', ['line.text', 'item.text', 'item.url']],
      ['src/features/pins/components/PinManager.vue', ['group', 'pin.label']],
      ['src/features/recording/components/RecordingPanel.vue', ['result.filename', 'result.path', 'item.path']],
      ['src/features/clipboard/components/ClipboardItem.vue', ['file.name', 'item.preview']],
    ] as const) {
      const template = parse(readFileSync(file, 'utf8')).descriptor.template!.content
      for (const value of expressions) expect(template, `${file}: ${value}`).toContain(`{{ ${value} }}`)
    }
    // 不从 DOM 扫描或替换文本；这一边界避免语言切换误改用户内容。
    expect(readFileSync('src/shared/i18n/i18n.ts', 'utf8')).not.toContain('MutationObserver')
    expect(readFileSync('src/shared/ui/TooltipLayer.vue', 'utf8')).not.toContain('t(text)')
    expect(readFileSync('src/features/clipboard/components/ClipboardItem.vue', 'utf8')).toContain(':title="file.path"')
  })
})
