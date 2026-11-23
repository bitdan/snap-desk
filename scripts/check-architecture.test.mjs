import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { tmpdir } from 'node:os'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { checkArchitecture } from './check-architecture.mjs'

function fixture(t, files) {
  const root = mkdtempSync(join(tmpdir(), 'snap-architecture-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  for (const [name, content] of Object.entries(files)) {
    const path = join(root, name)
    mkdirSync(dirname(path), { recursive: true })
    writeFileSync(path, content)
  }
  return checkArchitecture(root)
}

test('core crate 不依赖界面框架且内嵌资源存在', t => {
  assert.deepEqual(fixture(t, {
    'crates/snap-core/Cargo.toml': '[dependencies]\nserde = "1"\n# 不依赖 tauri = 或 gpui = 的注释不算\n',
    'crates/snap-core/src/lib.rs': '//! 与 tauri:: 无关的说明不算\npub mod settings;\nconst X: &str = include_str!("data.json");',
    'crates/snap-core/src/data.json': '{}',
  }), [])
  const issues = fixture(t, {
    'crates/snap-core/Cargo.toml': '[dependencies]\ntauri = "2"\n',
    'crates/snap-core/src/lib.rs': 'use gpui_kit::App;\nconst X: &str = include_str!("missing.sql");',
  })
  assert.ok(issues.some(issue => issue.startsWith('crates/snap-core/Cargo.toml') && issue.includes('不得依赖 Tauri 或 GPUI')))
  assert.ok(issues.some(issue => issue.startsWith('crates/snap-core/src/lib.rs') && issue.includes('不得依赖 Tauri 或 GPUI')))
  assert.ok(issues.some(issue => issue.includes('内嵌资源不存在 missing.sql')))
})

test('应用与工具可以依赖 GPUI，但内嵌资源和路径模块必须存在', t => {
  assert.deepEqual(fixture(t, { 'apps/desktop-gpui/src/main.rs': 'use gpui_kit::App;' }), [])
  const issues = fixture(t, {
    'apps/desktop-gpui/src/shell.rs': 'const ICON: &[u8] = include_bytes!("../../../resources/icons/32x32.png");',
    'tools/release-sign/src/main.rs': '#[path = "../old/sign.rs"] mod sign;',
  })
  assert.ok(issues.some(issue => issue.startsWith('apps/desktop-gpui/src/shell.rs') && issue.includes('内嵌资源不存在')))
  assert.ok(issues.some(issue => issue.startsWith('tools/release-sign/src/main.rs') && issue.includes('子模块路径不存在')))
})
