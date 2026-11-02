import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createReleaseNotes, notesFromTag } from './release-notes.mjs'

test('标签变动位于资源说明之前，保留 Markdown 并去掉重复版本标题', () => {
  const notes = createReleaseNotes('v0.2.0-beta.4', 'Snap Desk v0.2.0-beta.4\r\n\r\n- 修复签名配置。\r\n- 保留独立 EXE。', 'translation-engine-v1')
  assert.ok(notes.startsWith('## 更新内容\n\n- 修复签名配置。\n- 保留独立 EXE。'))
  assert.ok(notes.includes('翻译引擎：translation-engine-v1'))
  assert.ok(!notes.includes('Snap Desk v0.2.0-beta.4'))
})

test('没有变动说明时阻止发布，避免 Releases 只包含固定资源信息', () => {
  assert.throws(() => createReleaseNotes('v0.2.0', 'Snap Desk v0.2.0', 'engine'), /缺少更新内容/)
  assert.throws(() => createReleaseNotes('v0.2.0', '', 'engine'), /缺少更新内容/)
  assert.throws(() => createReleaseNotes('../bad-tag', '- 更新', 'engine'), /格式无效/)
})

test('读取现有附注标签中的实际修复内容', () => {
  const notes = notesFromTag('v0.2.0-beta.4')
  assert.ok(notes.includes('构建前验证更新签名密钥'))
  assert.ok(notes.includes('修复独立 EXE 被 NSIS'))
})
