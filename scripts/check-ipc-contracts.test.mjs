import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { tmpdir } from 'node:os'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { checkIpcContracts } from './check-ipc-contracts.mjs'

const pairs = { CapturePaint: 'CapturePaint', InputEffects: 'InputEffects', RecorderConfig: 'RecorderConfig', RecordingView: 'RecordingView', RecordingResult: 'RecordingResult', RecordingDirectory: 'RecordingDirectory', ExportSettings: 'ExportSettings', Settings: 'Settings', RecordingAnnotationView: 'AnnotationView' }
function fixture(t, changes = {}) {
  const root = mkdtempSync(join(tmpdir(), 'snap-ipc-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const files = {
    'src-tauri/src/app/mod.rs': 'tauri::generate_handler![capture::save_v2,]',
    'src-tauri/src/capture/mod.rs': '#[tauri::command] pub async fn save_v2(app: AppHandle, window: WebviewWindow, mut request_id: u64, optional: Option<String>) {}',
    'src-tauri/src/diagnostics/catalog.rs': 'pub const COMMANDS: &[&str] = &["save_v2"];',
    'src/services/ipc-commands.ts': 'export const ipcCommands = ["save_v2"] as const',
    'src/features/capture/api.ts': "export const captureApi = { save: (requestId: number) => invoke('save_v2', { requestId }) }",
    'src/features/capture/types.ts': Object.keys(pairs).map(name => `export interface ${name} { sampleValue: number }`).join('\n'),
    'src-tauri/src/capture/types.rs': Object.values(pairs).map(name => `#[serde(rename_all = "camelCase")] pub struct ${name} { pub sample_value: u32 }`).join('\n'),
    ...changes,
  }
  for (const [name, content] of Object.entries(files)) { const path = join(root, name); mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, content) }
  return checkIpcContracts(root)
}
test('支持数字命令名、可变参数、注入参数、可选参数和 camelCase 字段', t => assert.deepEqual(fixture(t), []))
test('命令遗漏或白名单重复时失败', t => {
  const issues = fixture(t, { 'src/services/ipc-commands.ts': 'export const ipcCommands = ["stale", "stale"]' })
  assert.ok(issues.some(issue => issue.includes('缺少 save_v2')))
  assert.ok(issues.some(issue => issue.includes('多余 stale')))
  assert.ok(issues.some(issue => issue.includes('存在重复项')))
  const duplicate = fixture(t, { 'src-tauri/src/app/mod.rs': 'tauri::generate_handler![capture::save_v2, capture::save_v2]' })
  assert.ok(duplicate.some(issue => issue.includes('原生命令注册: 存在重复项')))
  const missing = fixture(t, { 'src-tauri/src/capture/forgotten.rs': '#[tauri::command] fn forgotten() {}' })
  assert.ok(missing.some(issue => issue.includes('原生命令注册: 缺少 forgotten')))
})
test('请求字段漂移和跨模块重复 API 名不能静默通过', t => {
  const issues = fixture(t, {
    'src/features/capture/api.ts': "export const captureApi = { save: () => invoke('save_v2', { wrongName: 1 }) }",
    'src/features/pins/api.ts': 'export const pinsApi = { save: () => undefined }',
  })
  assert.ok(issues.some(issue => issue.includes('缺少参数 requestId')))
  assert.ok(issues.some(issue => issue.includes('未知参数 wrongName')))
  assert.ok(issues.some(issue => issue.includes('API 方法名: 存在重复项')))
})
test('关键结构新增字段时要求前端同步', t => {
  const direct = fixture(t, { 'src-tauri/src/capture/types.rs': Object.values(pairs).map(name => `pub struct ${name} { pub sample_value: u32, pub new_field: bool }`).join('\n') })
  assert.ok(direct.some(issue => issue.includes('缺少 newField')))
})
