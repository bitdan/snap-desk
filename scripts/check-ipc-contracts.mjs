import { readFileSync, readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = fileURLToPath(new URL('../', import.meta.url))
const files = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(directory, entry.name)) : [join(directory, entry.name)])
const read = file => readFileSync(file, 'utf8')
const camel = name => name.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase())
function balanced(source, start, open = '(', close = ')') {
  let depth = 1, end = start + 1
  for (; end < source.length && depth; end++) { if (source[end] === open) depth++; else if (source[end] === close) depth-- }
  return source.slice(start + 1, end - 1)
}
function splitFields(source) {
  let depth = 0, start = 0
  const fields = []
  for (let i = 0; i < source.length; i++) {
    if ('<({['.includes(source[i])) depth++
    else if ('>)}]'.includes(source[i])) depth--
    else if (source[i] === ',' && depth === 0) { fields.push(source.slice(start, i)); start = i + 1 }
  }
  fields.push(source.slice(start))
  return fields.map(field => field.trim()).filter(Boolean)
}
function parity(expected, actual, label, issues) {
  if (actual.length !== new Set(actual).size) issues.push(`${label}: 存在重复项`)
  for (const name of expected) if (!actual.includes(name)) issues.push(`${label}: 缺少 ${name}`)
  for (const name of actual) if (!expected.includes(name)) issues.push(`${label}: 多余 ${name}`)
}

/** 比较注册、调用、审计名单、参数名和关键结构字段，不依赖 Rust 编译或新增生成器。 */
export function checkIpcContracts(project = root) {
  const issues = [], invoked = [], methods = [], commandArgs = new Map(), tsTypes = new Map(), rustTypes = new Map()
  const nativeFiles = files(join(project, 'src-tauri/src')).filter(file => file.endsWith('.rs'))
  for (const file of nativeFiles) {
    const source = read(file).replace(/\/\/[^\n]*/g, '')
    const commands = /#\[tauri::command(?:\([^]*?\))?\]\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*\(/g
    for (const match of source.matchAll(commands)) {
      const args = splitFields(balanced(source, match.index + match[0].length - 1)).map(field => {
        const [name, ...type] = field.split(':')
        return { name: camel(name.trim().replace(/^mut\s+/, '')), type: type.join(':').trim() }
      }).filter(arg => !/\b(AppHandle|WebviewWindow|Window|State)\b/.test(arg.type))
      commandArgs.set(match[1], args)
    }
    for (const match of source.matchAll(/\bstruct\s+(\w+)\s*\{/g)) {
      const body = balanced(source, match.index + match[0].length - 1, '{', '}').replace(/#\[[^]*?\]/g, '')
      const names = splitFields(body).map(field => field.match(/^(?:pub(?:\([^)]*\))?\s+)?(\w+)\s*:/)?.[1]).filter(Boolean).map(camel)
      if (names.length) rustTypes.set(match[1], names)
    }
  }
  const bootstrap = read(join(project, 'src-tauri/src/app/mod.rs'))
  const handler = bootstrap.match(/generate_handler!\[([^]*?)\]/)?.[1] ?? ''
  const registered = handler.split(',').map(name => name.trim().split('::').at(-1)).filter(Boolean)
  if (!registered.length) issues.push('未找到原生命令注册表')
  parity([...commandArgs.keys()], registered, '原生命令注册', issues)
  for (const name of registered) if (!commandArgs.has(name)) issues.push(`注册命令没有对应实现: ${name}`)
  for (const file of files(join(project, 'src')).filter(file => file.endsWith('.ts'))) {
    const ast = ts.createSourceFile(file, read(file), ts.ScriptTarget.Latest, true)
    function visit(node) {
      if (ts.isInterfaceDeclaration(node)) tsTypes.set(node.name.text, node.members.filter(ts.isPropertySignature).map(member => member.name.getText(ast).replaceAll("'", '')))
      if (file.replaceAll('\\', '/').match(/\/features\/[^/]+\/api\.ts$/) && ts.isPropertyAssignment(node)
        && ts.isVariableDeclaration(node.parent.parent)) methods.push(node.name.getText(ast))
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'invoke' && ts.isStringLiteral(node.arguments[0])) {
        const name = node.arguments[0].text
        invoked.push(name)
        const args = []
        function objectKeys(value) {
          if (!value) return
          if (ts.isObjectLiteralExpression(value)) for (const field of value.properties) {
            if (ts.isPropertyAssignment(field) || ts.isShorthandPropertyAssignment(field)) args.push(field.name.getText(ast))
            else if (ts.isSpreadAssignment(field)) objectKeys(field.expression)
          }
          else if (ts.isParenthesizedExpression(value)) objectKeys(value.expression)
          else if (ts.isConditionalExpression(value)) { objectKeys(value.whenTrue); objectKeys(value.whenFalse) }
        }
        objectKeys(node.arguments[1])
        const native = commandArgs.get(name)
        if (!native) issues.push(`调用了未实现的命令: ${name}`)
        else {
          for (const arg of native) if (!args.includes(arg.name) && !/^Option\s*</.test(arg.type)) issues.push(`${name}: 缺少参数 ${arg.name}`)
          for (const arg of args) if (!native.some(value => value.name === arg)) issues.push(`${name}: 未知参数 ${arg}`)
        }
      }
      ts.forEachChild(node, visit)
    }
    visit(ast)
  }
  parity(registered, invoked, '前端命令调用', issues)
  parity(methods, methods, 'API 方法名', issues)
  const whitelist = read(join(project, 'src/services/ipc-commands.ts')).match(/ipcCommands\s*=\s*\[([^]*?)\]/)?.[1] ?? ''
  const catalog = read(join(project, 'src-tauri/src/diagnostics/catalog.rs')).match(/COMMANDS[^=]*=\s*&\[([^]*?)\]/)?.[1] ?? ''
  const strings = value => [...value.matchAll(/["']([a-z_][a-z_0-9]*)["']/g)].map(match => match[1])
  parity(registered, strings(whitelist), '前端命令白名单', issues)
  parity(registered, strings(catalog), '原生命令审计名单', issues)
  for (const [frontend, native] of Object.entries({ PinFrame: 'PinFrame', PinSummary: 'PinSummary', PinAppearance: 'PinAppearance', CapturePaint: 'CapturePaint', InputEffects: 'InputEffects', RecorderConfig: 'RecorderConfig', RecordingView: 'RecordingView', RecordingResult: 'RecordingResult', RecordingDirectory: 'RecordingDirectory', ExportSettings: 'ExportSettings', RecordingExportSettings: 'RecordingExportSettings', Settings: 'Settings', RecordingAnnotationView: 'AnnotationView' })) {
    if (!tsTypes.has(frontend) || !rustTypes.has(native)) { issues.push(`通信结构不存在: ${frontend}/${native}`); continue }
    parity(rustTypes.get(native), tsTypes.get(frontend), `通信字段 ${frontend}`, issues)
  }
  return [...new Set(issues)]
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const issues = checkIpcContracts()
  if (issues.length) { console.error(issues.join('\n')); process.exitCode = 1 }
  else console.log('IPC 注册、调用、参数、白名单及关键通信字段一致。')
}
