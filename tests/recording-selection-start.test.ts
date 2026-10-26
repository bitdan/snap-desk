import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { parse } from '@vue/compiler-sfc'
import ts from 'typescript'
import { recordingConfig, recordingRegion, recordingBitrate, defaultInputEffects } from '../src/lib/recording'
import { fromPixels } from '../src/lib/capture-targets'

// 执行选区页的真实开始入口，隔离原生 IPC；不启动 WebView、采集设备或编码器。
const script = parse(readFileSync('src/components/CaptureOverlay.vue', 'utf8')).descriptor.scriptSetup!.content
const ast = ts.createSourceFile('capture.ts', script, ts.ScriptTarget.Latest, true)
const entry = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'startRecording')!
const code = ts.transpileModule(entry.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
const chooseEntry = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'chooseRecordingSource')!
const chooseCode = ts.transpileModule(chooseEntry.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
const ref = <T>(value: T) => ({ value })
function fixture() {
  const calls: string[] = []
  const api = {
    startRecording: vi.fn(async () => { calls.push('prepare') }),
    setRecordingAnnotationMode: vi.fn(async () => { calls.push('same-window') }),
    controlRecording: vi.fn(async (_id: number, action: string) => { calls.push(action) }),
    selectRecordingWindow: vi.fn(),
  }
  const context = {
    api, recordingConfig, recordingRegion, recordingBitrate, fromPixels, disposed: false,
    recordingSourceChoice: { get value() { return context.recordingSource.value === 'window' ? `window:${context.selectedTarget.value}` : context.recordingSource.value } },
    marked: ref(false), viewport: ref({ width: 1920, height: 1080 }), selection: ref<any>(null), activeDisplay: ref({ x: 0, y: 0, width: 1920, height: 1080 }),
    recordingMode: ref(true), recordingCapabilities: ref({ available: true, systemAudio: false, microphone: false }),
    frame: ref<any>({ id: 7, width: 1920, height: 1080, originX: 0, originY: 0, displays: [{ x: 0, y: 0, width: 1920, height: 1080, scale: 1 }], targets: [] }),
    pixels: ref({ x: 10, y: 20, width: 640, height: 480 }), valid: ref(true), busy: ref(false), error: ref(''), gesture: ref(null),
    editor: { pending: false, interacting: false, history: { present: [] } },
    recordingSource: ref('region'), recordingFps: ref(60), captureCursor: ref(true), recordingCountdown: ref(0), recordingEncoding: ref('auto'), recordingQuality: ref('balanced'),
    recordingEffects: ref(defaultInputEffects()), recordingFormat: ref('mp4'), recordingMbps: ref(''), recordingSystemAudio: ref(false), recordingMicrophone: ref(false), selectedTarget: ref(null),
    liveRecordingId: ref<number | null>(null),
    releaseFrame: vi.fn(() => { calls.push('release-image') }),
    nextTick: async () => { calls.push('render-shared-workspace') },
  }
  api.selectRecordingWindow.mockImplementation(async () => ({ ...context.frame.value, previewRevision: 1 }))
  const start = new Function(...Object.keys(context), `${code}; return startRecording`)(...Object.values(context)) as () => Promise<void>
  const choose = new Function(...Object.keys(context), `${chooseCode}; return chooseRecordingSource`)(...Object.values(context)) as (event: Event | string) => Promise<void>
  return { context, calls, start, choose }
}

describe('选区工具栏准备录制', () => {
  it('图标菜单直接提交候选值，失败时保留原选择，不依赖已卸载的 DOM', async () => {
    const { context: c, choose } = fixture()
    c.frame.value.targets = [{ kind: 'window', rect: { x: 20, y: 30, width: 800, height: 600 } }]
    c.api.selectRecordingWindow.mockRejectedValueOnce(new Error('WINDOW_SELECT_ACTIVATE'))
    await choose('window:0')
    expect(c.recordingSource.value).toBe('region')
    expect(c.error.value).toContain('WINDOW_SELECT_ACTIVATE')
    expect(c.busy.value).toBe(false)
    await choose('window:0')
    expect(c.recordingSource.value).toBe('window')
  })
  it('从应用列表直接选择窗口并通过会话候选下标启动原生窗口来源', async () => {
    const { context: c, start, choose } = fixture()
    c.frame.value.targets = [{ kind: 'element' }, { kind: 'window', rect: { x: 20, y: 30, width: 800, height: 600 } }]
    await choose({ target: { value: 'window:1' } } as unknown as Event)
    expect(c.api.selectRecordingWindow).toHaveBeenCalledWith(7, 1)
    expect(c.frame.value.previewRevision).toBe(1)
    expect(c.recordingSource.value).toBe('window')
    expect(c.selectedTarget.value).toBe(1)
    expect(c.selection.value).toEqual({ x: 20, y: 30, width: 800, height: 600 })
    await start()
    expect(c.api.startRecording).toHaveBeenCalledWith(7, expect.anything(), expect.objectContaining({ source: 'selectedWindow', fps: 60 }), 1, [])
  })
  it('置前失败保留此前来源和画面，恢复下拉选择且不开始录制', async () => {
    const { context: c, choose } = fixture()
    c.frame.value.targets = [{ kind: 'window', rect: { x: 20, y: 30, width: 800, height: 600 } }]
    c.api.selectRecordingWindow.mockRejectedValueOnce(new Error('无法置前'))
    const event = { target: { value: 'window:0' } } as unknown as Event
    const original = c.frame.value
    await choose(event)
    expect(c.frame.value).toBe(original)
    expect(c.recordingSource.value).toBe('region')
    expect(c.error.value).toContain('无法置前')
    expect(c.busy.value).toBe(false)
    expect((event.target as HTMLSelectElement).value).toBe('region')
    expect(c.api.startRecording).not.toHaveBeenCalled()
  })
  it('切换应用返回的是新位置与尺寸时采用新几何，不恢复已经关闭的选区', async () => {
    const { context: c, choose } = fixture()
    const target = { kind: 'window', rect: { x: 20, y: 30, width: 800, height: 600 } }
    c.frame.value.targets = [target]
    const next = { ...c.frame.value, targets: [{ ...target, rect: { x: 40, y: 60, width: 1000, height: 700 } }] }
    c.api.selectRecordingWindow.mockResolvedValueOnce(next)
    await choose({ target: { value: 'window:0' } } as unknown as Event)
    expect(c.selection.value).toEqual(next.targets[0].rect)
    let release!: (value: typeof next) => void
    c.api.selectRecordingWindow.mockReturnValueOnce(new Promise(resolve => { release = resolve }))
    const task = choose({ target: { value: 'window:0' } } as unknown as Event)
    const newer = { ...next, id: 8 }
    c.frame.value = newer
    release(next); await task
    expect(c.frame.value).toBe(newer)
  })
  it('显示器来源实际更新范围，非法窗口下标和已有标注不修改选区', () => {
    const { context: c, choose } = fixture()
    choose({ target: { value: 'display' } } as unknown as Event)
    expect(c.selection.value).toEqual(c.activeDisplay.value)
    choose({ target: { value: 'window:99' } } as unknown as Event)
    expect(c.recordingSource.value).toBe('display')
    c.marked.value = true
    choose({ target: { value: 'region' } } as unknown as Event)
    expect(c.recordingSource.value).toBe('display')
  })
  it('确认范围只完成准备和原窗口接管，等待用户明确开始', async () => {
    const { context: c, calls, start } = fixture()
    await start()
    expect(calls).toEqual(['prepare', 'release-image', 'render-shared-workspace', 'same-window'])
    expect(c.liveRecordingId.value).toBe(7)
    expect(c.api.controlRecording).not.toHaveBeenCalled()
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
