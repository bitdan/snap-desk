// CI 校验标签，再从 package.json 生成 Cargo/npm 字段；Tauri 直接引用 package.json。
import { readVersion, syncVersion } from './sync-version.mjs'

const tag = process.env.GITHUB_REF_NAME
const version = readVersion()
if (tag !== `v${version}`) {
  throw new Error(`发布标签 ${tag ?? '(未设置)'} 必须是 v${version}，请检查 package.json version。`)
}
// 错误标签不会改写文件；正确标签只同步版本，不解析或升级依赖。
syncVersion()
console.log(`发布版本校验通过：${tag}`)
