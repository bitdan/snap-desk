import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import { recordingConfig, recordingRegion, recordingBitrate, defaultInputEffects } from '../src/lib/recording'

// 执行选区页的真实开始入口，隔离原生 IPC；不启动 WebView、采集设备或编码器。
const script = parse(readFileSync('src/components/CaptureOverlay.vue', 'utf8')).descriptor.scriptSetup!.content
const ast = ts.createSourceFile('capture.ts', script, ts.ScriptTarget.Latest, true)
const entry = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'startRecording')!
const code = ts.transpileModule(entry.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
const ref = <T>(value: T) => ({ value })
function fixture() {
  const calls: string[] = []
  const api = {
    startRecording: vi.fn(async () => { calls.push('prepare') }),
    setRecordingAnnotationMode: vi.fn(async () => { calls.push('same-window') }),
    controlRecording: vi.fn(async (_id: number, action: string) => { calls.push(action) }),
  }
  const context = {
    api, recordingConfig, recordingRegion, recordingBitrate,
    recordingMode: ref(true), recordingCapabilities: ref({ available: true, systemAudio: false, microphone: false }),
    frame: ref<any>({ id: 7, originX: 0, originY: 0, displays: [{ x: 0, y: 0, width: 1920, height: 1080, scale: 1 }], targets: [] }),
    pixels: ref({ x: 10, y: 20, width: 640, height: 480 }), valid: ref(true), busy: ref(false), error: ref(''), gesture: ref(null),
    editor: { pending: false, interacting: false, history: { present: [] } },
    recordingSource: ref('region'), recordingFps: ref(30), captureCursor: ref(true), recordingCountdown: ref(0), recordingEncoding: ref('auto'), recordingQuality: ref('balanced'),
    recordingEffects: ref(defaultInputEffects()), recordingFormat: ref('mp4'), recordingMbps: ref(''), recordingSystemAudio: ref(false), recordingMicrophone: ref(false), selectedTarget: ref(null),
    liveRecordingId: ref<number | null>(null),
    releaseFrame: vi.fn(() => { calls.push('release-image') }),
    nextTick: async () => { calls.push('render-shared-workspace') },
  }
  const start = new Function(...Object.keys(context), `${code}; return startRecording`)(...Object.values(context)) as () => Promise<void>
  return { context, calls, start }
}

describe('选区工具栏直接开始录制', () => {
  it('一次点击完成准备、原窗口接管和开始，不等待第二次点击', async () => {
    const { context: c, calls, start } = fixture()
    await start()
    expect(calls).toEqual(['prepare', 'release-image', 'render-shared-workspace', 'same-window', 'start'])
    expect(c.liveRecordingId.value).toBe(7)
    expect(c.api.controlRecording).toHaveBeenCalledWith(7, 'start')
    expect(c.busy.value).toBe(false)
    expect(c.error.value).toBe('')
  })
  it('准备失败保留原选区，不接管或开始', async () => {
    const { context: c, start } = fixture()
    c.api.startRecording.mockRejectedValueOnce(new Error('采集设备不可用'))
    await start()
    expect(c.releaseFrame).not.toHaveBeenCalled()
    expect(c.liveRecordingId.value).toBeNull()
    expect(c.api.controlRecording).not.toHaveBeenCalled()
    expect(c.error.value).toContain('采集设备不可用')
  })
  it('接管失败取消已准备的会话，不遗留等待开始的状态', async () => {
    const { context: c, start } = fixture()
    c.api.setRecordingAnnotationMode.mockRejectedValueOnce(new Error('窗口保护不可用'))
    await start()
    expect(c.api.controlRecording).toHaveBeenCalledTimes(1)
    expect(c.api.controlRecording).toHaveBeenCalledWith(7, 'cancel')
    expect(c.error.value).toContain('窗口保护不可用')
    expect(c.busy.value).toBe(false)
  })
  it('原生录制准备不再显示旧的 480×48 控制窗，选区窗口支持透明复用', () => {
    const native = readFileSync('src-tauri/src/recorder/manager.rs', 'utf8').split('pub async fn start_recording')[1].split('fn measured_capture')[0]
    expect(native).not.toContain('chrome::show_recording(')
    expect(native).not.toContain('while !recording.2')
    const config = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'))
    expect(config.app.windows.find((w: { label: string }) => w.label === 'capture').transparent).toBe(true)
  })
})
