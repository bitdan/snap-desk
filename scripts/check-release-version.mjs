// CI 在安装依赖前校验标签，再从唯一版本来源生成 Tauri / Cargo / npm 所需字段。
import { readVersion, syncVersion } from './sync-version.mjs'

const tag = process.env.GITHUB_REF_NAME
const version = readVersion()
if (tag !== `v${version}`) {
  throw new Error(`发布标签 ${tag ?? '(未设置)'} 必须是 v${version}，请检查 package.json version。`)
}
// 错误标签不会改写文件；正确标签只同步版本，不解析或升级依赖。
syncVersion()
console.log(`发布版本校验通过：${tag}`)
