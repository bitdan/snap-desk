import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref, shallowRef } from 'vue'
import { recordingInputRect, useRecordingAnnotationLayout } from '../../src/features/recording/use-recording-annotation-layout'
import type { RecordingAnnotationView } from '../../src/features/recording/recording-annotations'
import { defaultRecordingAnnotationTools } from '../helpers/recording-fixtures'

const lifecycle = vi.hoisted(() => ({ mount: [] as (() => void)[], cleanup: [] as (() => void)[] }))
const api = vi.hoisted(() => ({ setRecordingAnnotationRegions: vi.fn() }))
vi.mock('../../src/services/api', () => ({ api }))
vi.mock('vue', async original => ({ ...await original<typeof import('vue')>(),
  onMounted: (callback: () => void) => lifecycle.mount.push(callback),
  onUnmounted: (callback: () => void) => lifecycle.cleanup.push(callback),
}))
afterEach(() => { lifecycle.cleanup.splice(0).forEach(fn => fn()); lifecycle.mount.length = 0; vi.unstubAllGlobals(); vi.resetAllMocks() })

async function setup() {
  vi.stubGlobal('innerWidth', 1280); vi.stubGlobal('innerHeight', 720)
  vi.stubGlobal('window', new EventTarget())
  const menu = { getBoundingClientRect: () => ({ left: 900, top: 650, width: 500, height: 200 }) }
  let menus: typeof menu[] = [], mutate = () => {}, resize = () => {}
  vi.stubGlobal('document', { documentElement: {}, querySelectorAll: () => menus })
  const disconnect = vi.fn()
  vi.stubGlobal('MutationObserver', class { constructor(callback: () => void) { mutate = callback } observe() {} disconnect = disconnect })
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { resize = callback } observe() {} disconnect = disconnect })
  let next = 0
  const frames = new Map<number, FrameRequestCallback>()
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++next, callback); return next })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
  const rect = { left: 100, top: 200, width: 300, height: 100 }
  const toolbar = shallowRef({ getBoundingClientRect: () => rect } as HTMLElement)
  const view = ref<RecordingAnnotationView | null>({ id: 7, revision: 0, width: 640, height: 480, x: 100, y: 100,
    viewportWidth: 1920, viewportHeight: 1080, editing: true, tools: defaultRecordingAnnotationTools(), mediaTimeMs: 0,
    editor: { ready: true, selected: -1, pending: false, interacting: false, canUndo: false, canRedo: false, text: '', error: '' } })
  api.setRecordingAnnotationRegions.mockResolvedValue(undefined)
  const scope = effectScope(), fail = vi.fn()
  const layout = scope.run(() => useRecordingAnnotationLayout(view, toolbar, fail))!
  lifecycle.cleanup.push(() => scope.stop())
  lifecycle.mount.splice(0).forEach(fn => fn())
  async function flush() {
    await nextTick()
    for (let i = 0; i < 4; i++) {
      const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach(fn => fn(0))
      await Promise.resolve(); await nextTick()
    }
  }
  await flush()
  return { layout, view, rect, fail, flush, mutate: () => mutate(), resize: () => resize(), frames, disconnect,
    showMenu: () => { menus = [menu]; mutate() } }
}

describe('录屏浮动工具栏的原生区域', () => {
  it('按实际视口换算物理像素，菜单裁到显示器内，拒绝空区域', () => {
    expect(recordingInputRect({ x: -10, y: 20, width: 30, height: 40 }, { width: 100, height: 100 }, { width: 150, height: 150 }))
      .toEqual({ x: 0, y: 30, width: 30, height: 60 })
    expect(recordingInputRect({ x: 200, y: 0, width: 10, height: 20 }, { width: 100, height: 100 }, { width: 150, height: 150 })).toBeNull()
    expect(recordingInputRect({ x: NaN, y: 0, width: 10, height: 20 }, { width: 100, height: 100 }, { width: 150, height: 150 })).toBeNull()
  })
  it('工具栏与打开的 Portal 菜单共同发布，布局不变时不重复 IPC', async () => {
    const task = await setup()
    expect(api.setRecordingAnnotationRegions).toHaveBeenLastCalledWith(7, [{ x: 150, y: 300, width: 450, height: 150 }])
    task.showMenu(); await task.flush()
    expect(api.setRecordingAnnotationRegions).toHaveBeenLastCalledWith(7, [{ x: 150, y: 300, width: 450, height: 150 }, { x: 1350, y: 975, width: 570, height: 105 }])
    const count = api.setRecordingAnnotationRegions.mock.calls.length
    task.mutate(); await task.flush()
    expect(api.setRecordingAnnotationRegions).toHaveBeenCalledTimes(count)
    // 默认两行均在框下：先留 8px 间距、48px 录制控制，再留 8px 放标注栏。
    expect(parseFloat(String(task.layout.style.value.top))).toBeCloseTo((100 + 480) / 1.5 + 8 + 48 + 8)
    task.view.value!.y = 500; await task.flush()
    // 靠近底边时整体移到框上方，两行之间仍保留间距。
    expect(parseFloat(String(task.layout.style.value.top)) + task.rect.height + 8).toBeLessThanOrEqual(500 / 1.5)
    task.view.value!.y = 0; task.view.value!.height = 1080; await task.flush()
    // 全屏选区无法外置，两行一起限位，录制按钮不会与标注栏重叠。
    expect(parseFloat(String(task.layout.style.value.top))).toBeGreaterThanOrEqual(48 + 8 + 8)
    expect(parseFloat(String(task.layout.style.value.top)) + task.rect.height).toBeLessThanOrEqual(720 - 8)
  })
  it('关闭标注后停止发布，再次进入即使位置相同也重新开放工具栏', async () => {
    const task = await setup()
    task.view.value!.editing = false; await task.flush()
    const count = api.setRecordingAnnotationRegions.mock.calls.length
    task.rect.left = 200; task.mutate(); await task.flush()
    expect(api.setRecordingAnnotationRegions).toHaveBeenCalledTimes(count)
    task.view.value!.editing = true; await task.flush()
    expect(api.setRecordingAnnotationRegions).toHaveBeenCalledTimes(count + 1)
  })
  it('进行中的布局串行发送，拖动期间只保留最新位置，失败通知宿主', async () => {
    const task = await setup()
    let finish!: () => void
    api.setRecordingAnnotationRegions.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve }))
    task.rect.left = 200; task.mutate(); await task.flush()
    task.rect.left = 300; task.mutate(); await task.flush()
    task.rect.left = 400; task.mutate(); await task.flush()
    finish(); await task.flush()
    expect(api.setRecordingAnnotationRegions).toHaveBeenLastCalledWith(7, [{ x: 600, y: 300, width: 450, height: 150 }])
    api.setRecordingAnnotationRegions.mockRejectedValueOnce(new Error('窗口不可用'))
    task.rect.left = 500; task.mutate(); await task.flush()
    expect(task.fail).toHaveBeenCalledWith(expect.objectContaining({ message: '窗口不可用' }))
  })
})
