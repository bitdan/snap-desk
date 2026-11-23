import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const projectRoot = fileURLToPath(new URL('../', import.meta.url))
const slash = value => value.replaceAll('\\', '/')
/** 含 Rust 源码的 workspace 成员；内嵌资源（include_str! / include_bytes!）必须存在。 */
const rustRoots = ['crates/snap-core', 'apps/desktop-gpui', 'tools/release-sign']

function files(directory) {
  if (!existsSync(directory)) return []
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    if (entry.name === 'target') return []
    const path = join(directory, entry.name)
    return entry.isDirectory() ? files(path) : [path]
  })
}

/** 只读取源码，不编译 Rust；越界依赖或失效的内嵌资源会直接阻止 test。 */
export function checkArchitecture(root = projectRoot) {
  const issues = []
  const report = (file, message) => issues.push(`${file}: ${message}`)
  for (const base of rustRoots) {
    for (const file of files(join(root, base))) {
      const name = slash(relative(root, file))
      if (!/\.(rs|toml)$/.test(file)) continue
      const source = readFileSync(file, 'utf8')
      // 去掉 Rust `//` 与 TOML `#` 注释，说明文字里提到框架名不算；Rust 的 `#[...]` 属性保留。
      const code = source.replace(file.endsWith('.toml') ? /#[^\n]*/g : /\/\/[^\n]*/g, '')
      // snap-core 是与界面无关的业务核心，界面能力由调用方以闭包或 trait 注入。
      if (base === 'crates/snap-core' && /\b(tauri|gpui)[\w-]*\s*(::|=)/.test(code)) {
        report(name, 'snap-core 不得依赖 Tauri 或 GPUI，界面能力由调用方以闭包或 trait 注入')
      }
      for (const match of source.matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"/g)) {
        if (!existsSync(resolve(dirname(file), match[1]))) report(name, `内嵌资源不存在 ${match[1]}`)
      }
      // 显式路径模块同样参与检查，防止移动 Rust 文件后遗留旧位置。
      for (const match of source.matchAll(/#\[path\s*=\s*"([^"]+)"\]/g)) {
        if (!existsSync(resolve(dirname(file), match[1]))) report(name, `子模块路径不存在 ${match[1]}`)
      }
    }
  }
  return [...new Set(issues)]
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const issues = checkArchitecture()
  if (issues.length) {
    console.error(issues.join('\n'))
    process.exitCode = 1
  } else console.log('snap-core 边界与 Rust 内嵌资源检查通过。')
}
