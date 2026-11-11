import { expect, it } from 'vitest'
import { autoFinishAction } from '../../src/features/capture/after-action'
import { defaultSettings } from '../../src/features/settings/settings'

const state = { ctrlKey: false, recording: false, valid: true }

it('默认停留在工具栏，设置直接执行后框选完成即返回对应动作', () => {
  expect(defaultSettings().screenshotAfter).toBe('none')
  expect(autoFinishAction({ afterAction: 'none' }, state)).toBeNull()
  expect(autoFinishAction({}, state)).toBeNull()
  expect(autoFinishAction(null, state)).toBeNull()
  for (const action of ['copy', 'pin', 'save'] as const) expect(autoFinishAction({ afterAction: action }, state)).toBe(action)
})

it('按住 Ctrl、录屏、翻译截图、回放和空选区都不直接执行', () => {
  const frame = { afterAction: 'copy' as const }
  expect(autoFinishAction(frame, { ...state, ctrlKey: true })).toBeNull()
  expect(autoFinishAction(frame, { ...state, recording: true })).toBeNull()
  expect(autoFinishAction(frame, { ...state, valid: false })).toBeNull()
  expect(autoFinishAction({ ...frame, translation: true }, state)).toBeNull()
  expect(autoFinishAction({ ...frame, replay: { index: 0, total: 1, rect: { x: 0, y: 0, width: 1, height: 1 }, annotations: [] } }, state)).toBeNull()
})
