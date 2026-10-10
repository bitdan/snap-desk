// IDEA 启动后会保留当时的 PATH；随后安装 Rust，IDE 内的 npm 进程可能仍找不到 Cargo。
import { existsSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { homedir } from 'node:os'
import { delimiter, resolve } from 'node:path'

/** 保留当前工具链优先级，找不到 Cargo 时补入安装目录；仅影响子进程，不改系统环境。 */
export function rustEnvironment(source = process.env) {
  const env = { ...source }
  // Windows 环境变量名不区分大小写。统一为 PATH，避免 Path/PATH 同时存在导致子进程取错值。
  const pathKeys = Object.keys(env).filter(key => process.platform === 'win32' ? key.toLowerCase() === 'path' : key === 'PATH')
  const originalPath = pathKeys.map(key => env[key]).filter(Boolean).join(delimiter)
  for (const key of pathKeys) delete env[key]
  env.PATH = originalPath
  const probe = () => spawnSync('cargo', ['--version'], { env, encoding: 'utf8', windowsHide: true })
  const check = result => {
    if (result.status === 0) return true
    // Cargo 已找到但工具链损坏/未配置时，应报告真实错误，不能当成 PATH 缺失。
    if (result.error?.code !== 'ENOENT') {
      throw new Error(`Cargo 无法运行：${result.stderr?.trim() || result.error?.message || `退出码 ${result.status}`}`)
    }
    return false
  }
  if (check(probe())) return env

  const candidates = [source.CARGO_HOME && resolve(source.CARGO_HOME, 'bin'), resolve(homedir(), '.cargo', 'bin')].filter(Boolean)
  for (const bin of candidates) {
    if (!existsSync(resolve(bin, process.platform === 'win32' ? 'cargo.exe' : 'cargo'))) continue
    env.PATH = [bin, originalPath].filter(Boolean).join(delimiter)
    if (check(probe())) {
      console.log(`已补充 Rust 工具目录：${bin}`)
      return env
    }
  }
  throw new Error('找不到 Cargo。请安装 Rust，或设置 CARGO_HOME 指向 Rust 安装目录，然后重启 IDEA。')
}
