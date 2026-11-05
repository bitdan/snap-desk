import { describe, expect, it, vi } from 'vitest'

const bridge = vi.hoisted(() => ({ handler: null as null | ((event: { payload: unknown }) => void) }))
vi.mock('../../src/services/ipc', () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    bridge.handler = handler
    return () => {}
  },
}))

import { onSettingsChanged } from '../../src/features/settings/events'
import { defaultSettings } from '../../src/features/settings/settings'

describe('设置变更事件', () => {
  it('订阅者直接拿到完整配置，主界面无需再查询一次状态', async () => {
    const received: unknown[] = []
    await onSettingsChanged(value => received.push(value))
    const settings = defaultSettings()
    bridge.handler?.({ payload: settings })
    expect(received).toEqual([settings])
  })
})
