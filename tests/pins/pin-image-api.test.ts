import { expect, it, vi } from 'vitest'
const { invoke, listen, current } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), current: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: current }))
import { pinsApi } from '../../src/features/pins/api'
import { onPinVisibility, onPinView } from '../../src/features/pins/events'

it('读取携带已缓存版本，完整重读发送 null', async () => {
  await pinsApi.currentPin(3)
  expect(invoke).toHaveBeenLastCalledWith('current_pin', { knownRevision: 3 })
  await pinsApi.currentPin()
  expect(invoke).toHaveBeenLastCalledWith('current_pin', { knownRevision: null })
})
it('识别面板只调整自身窗口，关闭时请求恢复，不更改贴图缩放设置', async () => {
  await pinsApi.setPinRecognitionOpen(true)
  expect(invoke).toHaveBeenLastCalledWith('set_pin_recognition_open', { expanded: true })
  await pinsApi.setPinRecognitionOpen(false)
  expect(invoke).toHaveBeenLastCalledWith('set_pin_recognition_open', { expanded: false })
})
it('隐藏一张贴图只通知所属窗口，其他贴图不释放图片', async () => {
  const subscriptions: { target: string; callback: (event: { payload: boolean }) => void }[] = []
  listen.mockImplementation((_name, callback, options) => { subscriptions.push({ target: options.target, callback }); return Promise.resolve(vi.fn()) })
  const first = vi.fn(), second = vi.fn()
  current.mockReturnValueOnce({ label: 'pin-1' }).mockReturnValueOnce({ label: 'pin-2' })
  await onPinVisibility(first); await onPinVisibility(second)
  for (const subscription of subscriptions) if (subscription.target === 'pin-1') subscription.callback({ payload: false })
  expect(first).toHaveBeenCalledExactlyOnceWith(false)
  expect(second).not.toHaveBeenCalled()
})
it('穿透恢复状态只更新所属贴图，不改变其他窗口', async () => {
  current.mockReturnValueOnce({ label: 'pin-4' })
  listen.mockResolvedValueOnce(vi.fn())
  const updated = vi.fn()
  await onPinView(updated)
  expect(listen).toHaveBeenLastCalledWith('pin-view-changed', expect.any(Function), { target: 'pin-4' })
  const view = { zoom: 1, opacity: 1, locked: false, topmost: true, toolbarExpanded: true, clickThrough: false }
  listen.mock.calls.at(-1)![1]({ payload: view })
  expect(updated).toHaveBeenCalledExactlyOnceWith(view)
})
