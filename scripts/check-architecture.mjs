import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { parse } from '@vue/compiler-sfc'

const projectRoot = fileURLToPath(new URL('../', import.meta.url))
const frontendLayers = new Set(['app', 'features', 'services', 'shared'])
const nativeDomains = new Set(['app', 'capture', 'clipboard', 'diagnostics', 'pins', 'recognition', 'resources', 'settings', 'shared'])
const slash = value => value.replaceAll('\\', '/')

function files(directory) {
  if (!existsSync(directory)) return []
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? files(path) : [path]
  })
}

/** 只读取源码，不启动 Vite、不编译 Rust；新增根文件或越层依赖会直接阻止 check/test。 */
export function checkArchitecture(root = projectRoot) {
  const issues = []
  const report = (file, message) => issues.push(`${file}: ${message}`)
  for (const file of files(join(root, 'src'))) {
    const name = slash(relative(root, file))
    const parts = name.split('/')
    if (name !== 'src/main.ts' && !frontendLayers.has(parts[1])) report(name, '源码必须归入 app/features/services/shared，禁止恢复平铺目录')
    if (parts[1] === 'features' && parts.length < 4) report(name, '功能文件必须归入 features/<功能名>/')
    if (!/\.(ts|vue)$/.test(file)) continue
    const source = readFileSync(file, 'utf8')
    const scripts = file.endsWith('.vue')
      ? (() => { const { descriptor } = parse(source); return [descriptor.script?.content, descriptor.scriptSetup?.content].filter(Boolean) })()
      : [source]
    for (const script of scripts) {
      const ast = ts.createSourceFile(name, script, ts.ScriptTarget.Latest, true)
      function dependency(specifier) {
        if (/^@\/(lib|components)(\/|$)/.test(specifier)) report(name, `禁止旧目录引用 ${specifier}`)
        if (!specifier.startsWith('@/') && !specifier.startsWith('.')) return
        const target = specifier.startsWith('@/') ? resolve(root, 'src', specifier.slice(2)) : resolve(dirname(file), specifier)
        if (![target, `${target}.ts`, `${target}.vue`, `${target}.json`, join(target, 'index.ts')].some(existsSync)) report(name, `引用目标不存在 ${specifier}`)
        const targetName = slash(relative(root, target))
        if (parts[1] === 'shared' && /^src\/(app|features|services)\//.test(targetName)) report(name, 'shared 不得依赖应用、业务或 IPC 服务')
        if (parts[1] === 'features' && targetName.startsWith('src/app/')) report(name, '功能模块不得反向依赖 app 组装层')
        if (parts[1] === 'services' && name !== 'src/services/api.ts' && targetName.startsWith('src/features/')) report(name, '底层服务不得依赖功能实现，仅 api 汇总入口允许组合功能')
      }
      function visit(node) {
        if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
          const specifier = node.moduleSpecifier.text
          dependency(specifier)
          if (specifier === '@tauri-apps/api/core' && name !== 'src/services/ipc.ts') {
            const bindings = node.importClause?.namedBindings
            if (bindings && (ts.isNamespaceImport(bindings) || bindings.elements.some(item => (item.propertyName ?? item.name).text === 'invoke'))) report(name, '原生命令只能经过 services/ipc.ts 调用')
            if (ts.isExportDeclaration(node)) report(name, '禁止绕过 IPC 边界转发原生命令')
          }
          if (specifier === '@/services/ipc' && parts[1] === 'features' && !/\/(api|events)\.ts$/.test(name)) report(name, '功能的底层通信只放在 api.ts/events.ts')
        }
        if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword && ts.isStringLiteral(node.arguments[0])) {
          const specifier = node.arguments[0].text
          dependency(specifier)
          if (specifier === '@tauri-apps/api/core' && name !== 'src/services/ipc.ts') report(name, '禁止动态加载原生命令绕过 IPC 边界')
        }
        if (name === 'src/services/api.ts' && (ts.isFunctionDeclaration(node) || ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node))) report(name, 'API 汇总入口只组合与转发，命令和类型放回所属功能')
        ts.forEachChild(node, visit)
      }
      visit(ast)
    }
  }

  for (const file of files(join(root, 'src-tauri/src'))) {
    const name = slash(relative(root, file))
    const parts = name.split('/')
    if (parts.length === 3 && !['lib.rs', 'main.rs'].includes(parts[2])) report(name, 'Rust 根目录只保留 lib.rs/main.rs，新增代码放入所属功能')
    if (parts.length > 3 && !nativeDomains.has(parts[2])) report(name, '未知原生功能目录，新增领域时同步模块地图')
    if (!file.endsWith('.rs')) continue
    const source = readFileSync(file, 'utf8')
    for (const match of source.matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"/g)) {
      if (!existsSync(resolve(dirname(file), match[1]))) report(name, `内嵌资源不存在 ${match[1]}`)
    }
    for (const match of source.matchAll(/crate::([a-z_]+)\b/g)) {
      if (!nativeDomains.has(match[1])) report(name, `未知 crate 模块 ${match[1]}，请使用归属目录路径`)
    }
    // 显式路径模块同样参与检查，防止移动 Rust 文件后遗留旧位置。
    for (const match of source.matchAll(/#\[path\s*=\s*"([^"]+)"\]/g)) {
      if (!existsSync(resolve(dirname(file), match[1]))) report(name, `子模块路径不存在 ${match[1]}`)
    }
  }
  return [...new Set(issues)]
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const issues = checkArchitecture()
  if (issues.length) {
    console.error(issues.join('\n'))
    process.exitCode = 1
  } else console.log('目录边界、前端依赖与 Rust 内嵌资源检查通过。')
}
