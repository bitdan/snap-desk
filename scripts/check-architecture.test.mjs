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

test('接受按功能组织的源码和可解析的共享依赖', t => {
  assert.deepEqual(fixture(t, {
    'src/shared/geometry/point.ts': 'export type Point = { x: number }',
    'src/features/capture/types.ts': "export type { Point } from '@/shared/geometry/point'",
    'src-tauri/src/lib.rs': 'mod capture;',
    'src-tauri/src/capture/mod.rs': 'pub fn start() {}',
  }), [])
})

test('阻止平铺目录、反向依赖和失效路径（包含 Vue 与动态导入）', t => {
  const issues = fixture(t, {
    'src/lib/loose.ts': 'export const value = 1',
    'src/shared/ui/Bad.vue': "<script setup>import '@/features/capture/types'</script><template />",
    'src/features/capture/types.ts': "export * from '@/features/pins/missing'; import('@/lib/loose')",
  })
  assert.ok(issues.some(issue => issue.includes('禁止恢复平铺目录')))
  assert.ok(issues.some(issue => issue.includes('shared 不得依赖')))
  assert.ok(issues.some(issue => issue.includes('引用目标不存在')))
  assert.ok(issues.some(issue => issue.includes('禁止旧目录引用')))
})

test('阻止直接、别名、命名空间与动态方式绕过命令边界', t => {
  for (const content of [
    "import { invoke } from '@tauri-apps/api/core'",
    "import { invoke as call } from '@tauri-apps/api/core'",
    "import * as core from '@tauri-apps/api/core'",
    "const core = import('@tauri-apps/api/core')",
  ]) assert.ok(fixture(t, { 'src/features/capture/api.ts': content }).length > 0)
})

test('阻止原生根目录堆积、旧 crate 路径与失效内嵌资源', t => {
  const issues = fixture(t, {
    'src-tauri/src/loose.rs': 'use crate::translation::AiSettings;',
    'src-tauri/src/recognition/mod.rs': 'const MODEL: &str = include_str!("../missing.json");',
    'src-tauri/src/capture/mod.rs': '#[path = "../recorder/manager.rs"] mod recording;',
  })
  assert.ok(issues.some(issue => issue.includes('Rust 根目录只保留')))
  assert.ok(issues.some(issue => issue.includes('未知 crate 模块 translation')))
  assert.ok(issues.some(issue => issue.includes('内嵌资源不存在')))
  assert.ok(issues.some(issue => issue.includes('子模块路径不存在')))
})
