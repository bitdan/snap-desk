import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Vue from 'vue'
import { clientComponent, MemoryNode as Node, memoryRenderer as renderer } from '../helpers/component-host'
import { h, nextTick } from 'vue'
import type { RecordingView } from '../../src/features/recording/recording'

const api = vi.hoisted(() => ({ currentRecording: vi.fn(), latestRecordingResult: vi.fn(), controlRecording: vi.fn(), selectRecordingArea: vi.fn(), setRecordingAnnotationMode: vi.fn(), updateRecordingAnnotations: vi.fn(), recordingAnnotations: vi.fn(), setRecordingAnnotationTools: vi.fn(), recordingAnnotationEditorAction: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
import RecordingControls from '../../src/features/recording/components/RecordingControls.vue'
// Vitest 的 Node 管线保留真实 setup，模板另编译为客户端 render 供内存宿主执行。
const RecordingControlsClient = clientComponent(RecordingControls, new URL('../../src/features/recording/components/RecordingControls.vue', import.meta.url))
const Wrapper = { setup() { return () => h(RecordingControlsClient) } }
const node = (type: string) => new Node(type)
const root = node('root')
function find(predicate: (node: Node) => boolean, current = root): Node | undefined {
  if (predicate(current)) return current
  for (const child of current.children) { const found = find(predicate, child); if (found) return found }
}
function view(phase: RecordingView['phase'], id = 7): RecordingView {
  return { id, revision: 1, phase, seconds: 3, frames: 90, targetFps: 30, systemAudio: false, microphone: false,
    droppedFrames: 0, encodingMillis: 0, targetBitrate: 8000000, encoderKind: 'software', encoderFallback: null,
    countdownRemaining: 0, message: '', path: phase === 'completed' ? 'D:/video.mp4' : null,
    stats: { throttledSamples: 0, queuedFrames: 0, capturePolls: 0, capturedFrames: 90, captureMillis: 0, captureCopyBytes: 0, compositionCopyBytes: 0,
      composedFrames: 90, compositionMillis: 0, queueDroppedFrames: 0, staleQueuedFrames: 0, nv12OutputBytes: 0,
      recentFps: 30, rejectedCaptureFrames: 0, rejectedComposedFrames: 0 } }
}
async function flush() { await vi.advanceTimersByTimeAsync(0); await nextTick() }
async function mount() { renderer.render(h(Wrapper), root); await flush() }

beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks()
  vi.stubGlobal('localStorage', {getItem:()=>null,setItem:vi.fn()})
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() })
  api.recordingAnnotations.mockResolvedValue(null)
  api.currentRecording.mockResolvedValue(view('completed'))
  api.latestRecordingResult.mockResolvedValue({ id: 42, path: 'D:/video.mp4', filename: 'video.mp4' })
  api.controlRecording.mockResolvedValue(undefined)
  api.selectRecordingArea.mockResolvedValue(undefined)
})
afterEach(() => { renderer.render(null, root); vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals() })

describe('录屏控制条直接结束并保存', () => {
  it.each(['ready', 'countdown', 'recording', 'paused'] as const)('%s 阶段通过独立控制条进入标注', async phase => {
    api.currentRecording.mockResolvedValue(view(phase))
    await mount()
    const annotate = find(node => node.props['aria-label'] === '录制中标注')!
    expect(annotate.props.disabled).toBe(false)
    ;(annotate.props.onClick as () => void)(); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenCalledWith(7, true)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('标注窗口打开失败时显示错误并允许重试', async () => {
    api.currentRecording.mockResolvedValue(view('recording'))
    api.setRecordingAnnotationMode.mockRejectedValueOnce(new Error('标注窗口不可用'))
    await mount()
    const annotate = find(node => node.props['aria-label'] === '录制中标注')!
    ;(annotate.props.onClick as () => void)(); await flush()
    expect(find(node => node.props.role === 'alert')?.text).toContain('标注窗口不可用')
    expect(annotate.props.disabled).toBe(false)
  })
  it('标注时控制窗只显示录制操作，绘图工具交由选区覆盖层', async () => {
    api.currentRecording.mockResolvedValue(view('recording'))
    api.recordingAnnotations.mockResolvedValue({id:7,revision:0,editing:true})
    await mount()
    expect(find(node => String(node.props.class).includes('annotation-workspace-toolbar'))).toBeUndefined()
    expect(find(node => node.props['aria-label'] === '标注保留时间')).toBeUndefined()
    expect(find(node => node.props['aria-label'] === '暂停录制')).toBeDefined()
  })
  it('工具窗的撤销快捷键也转发到同一个画布编辑器',async()=>{
    api.currentRecording.mockResolvedValue(view('recording'))
    api.recordingAnnotations.mockResolvedValue({id:7,revision:0,editing:true})
    await mount()
    const handler=vi.mocked(window.addEventListener).mock.calls.find(call=>call[0]==='keydown')![1] as (e:KeyboardEvent)=>void
    const event={key:'z',ctrlKey:true,preventDefault:vi.fn(),target:{closest:()=>null}} as unknown as KeyboardEvent
    handler(event);await flush()
    expect(event.preventDefault).toHaveBeenCalled()
    expect(api.recordingAnnotationEditorAction).toHaveBeenCalledWith(7,{kind:'undo'})
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('工具窗绘制时 Esc 先完成标注，不停止录屏', async () => {
    api.currentRecording.mockResolvedValue(view('recording'))
    api.recordingAnnotations.mockResolvedValue({id:7,revision:0,editing:true})
    await mount()
    const handler=vi.mocked(window.addEventListener).mock.calls.find(call=>call[0]==='keydown')![1] as (e:KeyboardEvent)=>void
    handler({key:'Escape',preventDefault:vi.fn()} as unknown as KeyboardEvent); await flush()
    expect(api.setRecordingAnnotationMode).toHaveBeenCalledWith(7,false)
    expect(api.controlRecording).not.toHaveBeenCalled()
  })
  it('控制条清空标注时不改变录屏阶段', async () => {
    api.currentRecording.mockResolvedValue(view('recording'))
    await mount()
    const clear = find(node => node.props['aria-label'] === '清空标注')!
    ;(clear.props.onClick as () => void)(); await flush()
    expect(api.recordingAnnotationEditorAction).toHaveBeenCalledWith(7,{kind:'clear'})
    expect(api.controlRecording).not.toHaveBeenCalled()
    expect(api.setRecordingAnnotationMode).not.toHaveBeenCalled()
  })
  it('停止后经过封装进入完成状态，收起控制条且不加载或再次导出视频', async () => {
    api.currentRecording.mockResolvedValue(view('recording'))
    await mount()
    api.currentRecording.mockResolvedValue({...view('encoding'),revision:2})
    find(node => node.props['aria-label'] === '结束录制并保存')!.props.onClick(); await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(7,'stop')
    expect(find(node => node.props['aria-label'] === '录屏控制条')).toBeDefined()
    api.currentRecording.mockResolvedValue({...view('completed'),revision:3})
    await vi.advanceTimersByTimeAsync(1000)
    expect(find(node => node.props['aria-label'] === '录屏控制条')).toBeUndefined()
    expect(find(node => node.type === 'video')).toBeUndefined()
    expect(api.latestRecordingResult).not.toHaveBeenCalled()
    expect(api.controlRecording).toHaveBeenCalledTimes(1)
  })
  it.each(['completed','cancelled','failed'] as const)('%s 终态不渲染播放器或二次编辑界面', async phase => {
    api.currentRecording.mockResolvedValue(view(phase))
    await mount()
    expect(find(node => node.type === 'main' || node.type === 'video')).toBeUndefined()
    expect(api.latestRecordingResult).not.toHaveBeenCalled()
  })
  it.each([['ready', 'start'], ['recording', 'pause'], ['paused', 'resume']] as const)('%s 状态支持空格控制', async (phase, action) => {
    api.currentRecording.mockResolvedValue(view(phase))
    await mount()
    const handler = vi.mocked(window.addEventListener).mock.calls.find(call => call[0] === 'keydown')![1] as (event: KeyboardEvent) => void
    const event = { key: ' ', preventDefault: vi.fn(), target: { closest: () => null } } as unknown as KeyboardEvent
    handler(event); await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(7, action)
    expect(event.preventDefault).toHaveBeenCalledOnce()
  })
  it.each(['recording', 'paused', 'countdown'] as const)('%s 时后台控制页不再展开另一套界面或触发二次开始', async phase => {
    api.currentRecording.mockResolvedValue(view(phase))
    await mount()
    await vi.advanceTimersByTimeAsync(1000)
    expect(api.setRecordingAnnotationMode).not.toHaveBeenCalled()
    expect(api.controlRecording).not.toHaveBeenCalled()
    expect(find(node => node.props['aria-label'] === '返回标注')).toBeUndefined()
  })
  it('常驻控制窗再次接到新会话时恢复开始按钮', async () => {
    await mount()
    api.currentRecording.mockResolvedValue(view('ready',8))
    await vi.advanceTimersByTimeAsync(10000)
    const start=find(node => node.props['aria-label'] === '开始录制')!
    expect(start).toBeDefined()
    start.props.onClick(); await flush()
    expect(api.controlRecording).toHaveBeenCalledWith(8,'start')
  })
})
