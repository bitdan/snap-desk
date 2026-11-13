// 从附注标签读取人工整理的变动内容，作为 Releases 和应用内更新说明的共同来源。
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../', import.meta.url))
export function createReleaseNotes(tag, annotation, engineTag) {
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag)) throw new Error('发布标签格式无效。')
  const lines = annotation.trim().split(/\r?\n/)
  if ([tag, `Snap Desk ${tag}`].includes(lines[0]?.trim())) lines.shift()
  const changes = lines.join('\n').trim()
  if (!changes) throw new Error('标签缺少更新内容，请使用 git tag -a 并在标签注释中填写本次变动。')
  return `## 更新内容\n\n${changes}\n\n## 下载与资源\n\n- Windows x64：NSIS 安装包（-setup.exe）或独立 EXE（-standalone.exe），功能完整。\n- macOS Apple 芯片（预览）：DMG；未经 Apple 公证，首次打开需在“系统设置 → 隐私与安全性”中放行。\n- Linux x64（预览）：AppImage 或 deb；截图需要 X11 会话，AppImage 支持在线更新。\n- macOS 与 Linux 预览版目前提供截图、标注、贴图、识别与翻译，录屏、滚动截图和剪贴板历史暂未适配。\n- .app.tar.gz、.sig 与 updater.json 供在线更新使用，无需手动下载。\n- 翻译引擎：${engineTag}（Windows x64 CPU，协议 1）。\n- 请在“识别与翻译”设置中下载或导入翻译引擎与语言包，各分发形式均按需安装这些资源。\n- 模型来源和许可见各模型包内的 pack.json、LICENSE.txt。\n`
}

export function notesFromTag(tag, repository = root) {
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag)) throw new Error('发布标签格式无效。')
  const git = args => execFileSync('git', args, { cwd: repository, encoding: 'utf8', windowsHide: true }).trim()
  const ref = `refs/tags/${tag}`
  if (git(['cat-file', '-t', ref]) !== 'tag') throw new Error('发布必须使用带更新说明的附注标签：git tag -a。')
  const annotation = git(['for-each-ref', '--format=%(contents)', ref])
  const engine = JSON.parse(readFileSync(resolve(repository, 'scripts/ci/translation-engine.json'), 'utf8'))
  return createReleaseNotes(tag, annotation, engine.tag)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [tag, output] = process.argv.slice(2)
  if (!tag || !output) throw new Error('用法：node scripts/release-notes.mjs <tag> <output>')
  writeFileSync(output, notesFromTag(tag), 'utf8')
  console.log(`已从 ${tag} 标签注释生成更新说明。`)
}
