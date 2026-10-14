// package.json 是唯一版本来源；其他工具所需的版本字段由此脚本生成。
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const defaultRoot = fileURLToPath(new URL('../', import.meta.url))

/** 读取并校验版本，支持正式版及 alpha.1 / beta.1 / rc.1 等预发布版本。 */
export function readVersion(root = defaultRoot) {
  const { version } = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'))
  const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?$/
  if (typeof version !== 'string' || !semver.test(version)) {
    throw new Error('package.json version 必须符合 主版本.次版本.修订版本[-预发布版本] 格式。')
  }
  return version
}

/**
 * 开发、打包和 CI 发布前调用，只更新本项目版本，保留依赖版本和锁定信息。
 * 先计算并检查所有文件，再写入变化，避免结构异常时只同步了一部分文件。
 * root 可指定临时目录，供定向测试使用，不需要启动 Tauri 或 Rust 编译。
 */
export function syncVersion(root = defaultRoot) {
  const version = readVersion(root)
  const changes = []
  const update = (file, transform) => {
    const path = resolve(root, file)
    const before = readFileSync(path, 'utf8')
    const after = transform(before)
    if (after !== before) changes.push({ path, after })
  }
  const updateJson = (file, transform) => update(file, text => {
    const value = JSON.parse(text)
    transform(value)
    const newline = text.includes('\r\n') ? '\r\n' : '\n'
    return `${JSON.stringify(value, null, 2)}\n`.replace(/\n/g, newline)
  })
  updateJson('package-lock.json', lock => {
    if (!lock.packages?.['']) throw new Error('package-lock.json 缺少根包信息。')
    lock.version = version
    lock.packages[''].version = version
  })
  updateJson('src-tauri/tauri.conf.json', config => { config.version = version })

  // 按 TOML 段定位，禁止全局替换，以免改到恰好同版本的第三方依赖。
  const updateToml = (file, header) => update(file, text => {
    const sections = text.split(header)
    const indexes = sections.flatMap((section, index) =>
      index > 0 && /^name\s*=\s*"snap-desk"\s*$/m.test(section) ? [index] : [])
    if (indexes.length !== 1) throw new Error(`${file} 必须包含唯一的本项目 package 段。`)
    const index = indexes[0]
    if (!/^version\s*=\s*"[^"]+"/m.test(sections[index])) {
      throw new Error(`${file} 本项目 package 段缺少 version。`)
    }
    sections[index] = sections[index].replace(/^(version\s*=\s*")[^"]+(".*)$/m,
      (_, prefix, suffix) => `${prefix}${version}${suffix}`)
    return sections.join('')
  })
  // 捕获组保留原段头和换行，依赖段内容保持不变。
  updateToml('src-tauri/Cargo.toml', /(^\[[^\r\n]+\][^\r\n]*\r?\n)/m)
  updateToml('src-tauri/Cargo.lock', /(^\[\[package\]\]\r?\n)/m)

  for (const { path, after } of changes) writeFileSync(path, after, 'utf8')
  console.log(`版本已同步：${version}（更新 ${changes.length} 个文件）`)
  return version
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) syncVersion()
