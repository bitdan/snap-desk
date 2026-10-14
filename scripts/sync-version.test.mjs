// 使用隔离的临时项目验证版本同步，不运行 Rust / Tauri 构建。
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, copyFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve, dirname } from 'node:path'
import { spawnSync } from 'node:child_process'
import { syncVersion } from './sync-version.mjs'

function fixture(t, version) {
  const root = mkdtempSync(resolve(tmpdir(), 'snap-version-'))
  t.after(() => {
    // 清理范围必须是本测试在系统临时目录下创建的独立目录。
    assert.equal(dirname(root), resolve(tmpdir()))
    rmSync(root, { recursive: true })
  })
  mkdirSync(resolve(root, 'src-tauri'))
  mkdirSync(resolve(root, 'scripts'))
  const write = (file, value) => writeFileSync(resolve(root, file), value)
  const read = file => readFileSync(resolve(root, file), 'utf8')
  write('package.json', JSON.stringify({ version }))
  write('package-lock.json', JSON.stringify({ version: '0.1.3', packages: {
    '': { name: 'snap-desk', version: '0.1.3' },
    'node_modules/example': { version: '0.1.3', integrity: 'unchanged' },
  } }))
  write('src-tauri/tauri.conf.json', '{"version":"0.1.3","productName":"Snap Desk"}')
  write('src-tauri/Cargo.toml', '[package]\r\nname = "snap-desk"\r\nversion = "0.1.3"\r\n\r\n[dependencies]\r\nexample = "0.1.3"\r\n')
  write('src-tauri/Cargo.lock', 'version = 4\n\n[[package]]\nname = "example"\nversion = "0.1.3"\n\n[[package]]\nname = "snap-desk"\nversion = "0.1.3"\ndependencies = ["example"]\n')
  for (const file of ['sync-version.mjs', 'check-release-version.mjs']) {
    copyFileSync(new URL(file, import.meta.url), resolve(root, 'scripts', file))
  }
  const files = ['package.json', 'package-lock.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']
  return { root, write, read, snapshot: () => files.map(read) }
}

for (const version of ['0.1.4', '0.1.4-beta.1', '0.1.4-rc.1']) {
  test(`同步 ${version}，保留依赖和换行，重复执行不改变文件`, t => {
    const f = fixture(t, version)
    syncVersion(f.root)
    const lock = JSON.parse(f.read('package-lock.json'))
    assert.equal(lock.version, version)
    assert.equal(lock.packages[''].version, version)
    assert.deepEqual(lock.packages['node_modules/example'], { version: '0.1.3', integrity: 'unchanged' })
    assert.equal(JSON.parse(f.read('src-tauri/tauri.conf.json')).version, version)
    assert.equal(f.read('src-tauri/Cargo.toml'), `[package]\r\nname = "snap-desk"\r\nversion = "${version}"\r\n\r\n[dependencies]\r\nexample = "0.1.3"\r\n`)
    assert.equal(f.read('src-tauri/Cargo.lock'), `version = 4\n\n[[package]]\nname = "example"\nversion = "0.1.3"\n\n[[package]]\nname = "snap-desk"\nversion = "${version}"\ndependencies = ["example"]\n`)
    const before = f.snapshot()
    syncVersion(f.root)
    assert.deepEqual(f.snapshot(), before)
  })
}

test('非法版本和缺失 Cargo 包信息均在写入前失败', t => {
  const f = fixture(t, 'v0.1.4')
  let before = f.snapshot()
  assert.throws(() => syncVersion(f.root))
  assert.deepEqual(f.snapshot(), before)
  f.write('package.json', '{"version":"0.1.4"}')
  f.write('src-tauri/Cargo.lock', 'version = 4\n')
  before = f.snapshot()
  assert.throws(() => syncVersion(f.root))
  assert.deepEqual(f.snapshot(), before)
})

test('CI 先拒绝错误标签，匹配标签后再同步派生文件', t => {
  const f = fixture(t, '0.1.4-beta.1')
  const before = f.snapshot()
  const check = tag => spawnSync(process.execPath, [resolve(f.root, 'scripts/check-release-version.mjs')], {
    env: { ...process.env, GITHUB_REF_NAME: tag }, encoding: 'utf8',
  })
  assert.notEqual(check('v0.1.4').status, 0)
  assert.deepEqual(f.snapshot(), before)
  const result = check('v0.1.4-beta.1')
  assert.equal(result.status, 0, result.stderr)
  assert.equal(JSON.parse(f.read('src-tauri/tauri.conf.json')).version, '0.1.4-beta.1')
})
