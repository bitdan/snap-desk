import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test } from 'node:test'

const installer = readFileSync(new URL('../docs/install.sh', import.meta.url), 'utf8')
const fixture = 'AppImage fixture'
const url = 'https://github.com/bitdan/snap-desk/releases/download/v3.12.2/snap-desk-v3.12.2-linux-x64.AppImage'
const digest = createHash('sha256').update(fixture).digest('hex')

function runInstaller(t, hash) {
  const git = process.platform === 'win32' ? spawnSync('git', ['--exec-path'], { encoding: 'utf8' }) : null
  const bash = git?.status === 0 ? resolve(git.stdout.trim(), '../../../bin/bash.exe') : '/bin/bash'
  if (!existsSync(bash)) return t.skip('Bash is unavailable')
  const cwd = mkdtempSync(join(tmpdir(), 'snap-desk-install-test-'))
  t.after(() => rmSync(cwd, { recursive: true, force: true }))
  const harness = [
    'export HOME="$PWD"',
    'uname() { case "$1" in -s) echo Linux;; -m) echo x86_64;; esac; }',
    'curl() { case "${!#}" in',
    '  */downloads.tsv) printf "appimage\\t%s\\t%s\\n" "$test_url" "$test_hash";;',
    '  "$test_url") printf "%s" "$test_fixture" > "${@: -2:1}";;',
    '  *) return 22;;',
    'esac; }',
  ].join('\n')
  const result = spawnSync(bash, ['-s'], {
    cwd,
    env: { ...process.env, test_url: url, test_hash: hash, test_fixture: fixture },
    input: harness + '\n' + installer,
    encoding: 'utf8',
  })
  assert.ifError(result.error)
  return { result, path: join(cwd, '.local', 'bin', 'snap-desk.AppImage') }
}

test('安装命令校验 AppImage 后放到用户目录', t => {
  const run = runInstaller(t, digest)
  if (!run) return
  assert.equal(run.result.status, 0, run.result.stderr)
  assert.equal(readFileSync(run.path, 'utf8'), fixture)
  if (process.platform !== 'win32') assert.ok(statSync(run.path).mode & 0o100)
})

test('哈希不符时安装命令拒绝写入 AppImage', t => {
  const run = runInstaller(t, '0'.repeat(64))
  if (!run) return
  assert.notEqual(run.result.status, 0)
  assert.match(run.result.stderr, /SHA-256/)
  assert.ok(!existsSync(run.path))
})
