import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
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

// 在临时仓库里造标签，不依赖远端仍保留某个历史标签；旧标签被清理后 CI 也能通过。
test('读取附注标签中的更新内容，轻量标签被拒绝', t => {
  const repository = mkdtempSync(join(tmpdir(), 'snap-desk-release-notes-'))
  t.after(() => rmSync(repository, { recursive: true, force: true }))
  const identity = ['-c', 'user.name=test', '-c', 'user.email=test@example.invalid', '-c', 'commit.gpgsign=false', '-c', 'tag.gpgsign=false']
  const git = (...args) => execFileSync('git', [...identity, ...args], { cwd: repository, windowsHide: true })
  mkdirSync(join(repository, 'scripts/ci'), { recursive: true })
  writeFileSync(join(repository, 'scripts/ci/translation-engine.json'), JSON.stringify({ tag: 'translation-engine-test' }))
  // 标签注释经文件传入，避免 Windows 命令行参数的编码差异。
  const message = join(repository, 'tag-message.txt')
  writeFileSync(message, ['Snap Desk v0.2.0-beta.4', '', '- 构建前验证更新签名密钥。', '- 修复独立 EXE 被 NSIS 安装包覆盖。', ''].join('\n'), 'utf8')
  git('init', '-q')
  git('commit', '-q', '--allow-empty', '-m', 'init')
  git('tag', '-a', 'v0.2.0-beta.4', '-F', message)
  git('tag', 'v0.2.0-beta.5')

  const notes = notesFromTag('v0.2.0-beta.4', repository)
  assert.ok(notes.includes('构建前验证更新签名密钥'))
  assert.ok(notes.includes('修复独立 EXE 被 NSIS'))
  assert.ok(notes.includes('翻译引擎：translation-engine-test'))
  assert.ok(!notes.includes('Snap Desk v0.2.0-beta.4'))
  assert.throws(() => notesFromTag('v0.2.0-beta.5', repository), /附注标签/)
})
