// Tauri 会在打包时原地修改主程序的安装类型，必须在 beforeBundleCommand 中保留独立 EXE。
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readVersion } from './sync-version.mjs'

const unknown = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK')
const nsis = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS')
export function preserveStandalone(source, destination) {
  const binary = readFileSync(source)
  const offset = binary.indexOf(unknown)
  if (offset !== -1 && binary.indexOf(unknown, offset + unknown.length) === -1) {
    writeFileSync(destination, binary)
    return
  }
  // 签名失败后重试打包时，Cargo 可能复用已被标记的主程序；仅复用完全匹配的原始副本。
  if (existsSync(destination)) {
    const original = readFileSync(destination)
    const position = original.indexOf(unknown)
    if (position !== -1 && original.indexOf(unknown, position + unknown.length) === -1) {
      nsis.copy(original, position)
      if (original.equals(binary)) return
    }
  }
  throw new Error('无法确认独立 EXE 的原始安装标记，请重新构建主程序后再打包。')
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const source = process.env.SNAP_DESK_BUILD_EXECUTABLE
  if (!source) throw new Error('缺少 SNAP_DESK_BUILD_EXECUTABLE，请使用 desktop:package 打包。')
  const destination = join(dirname(source), `snap-desk-v${readVersion()}-windows-x64-standalone.exe`)
  preserveStandalone(source, destination)
  console.log('已在 NSIS 打包前保留独立 EXE。')
}
