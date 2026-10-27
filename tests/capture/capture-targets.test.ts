import { describe, expect, it } from 'vitest'
import { displayBounds, fromPixels, snapPoint, targetAt } from '../../src/features/capture/capture-targets'
import { toPixels } from '../../src/shared/geometry/selection'
import type { CaptureTarget } from '../../src/services/api'
import { parseRecentSelection, restoreRecentSelection } from '../../src/features/capture/recent-selection'
import { defaultSettings } from '../../src/features/settings/settings'

describe('桌面目标与混合 DPI 坐标', () => {
  const targets: CaptureTarget[] = [
    { kind: 'window', windowIndex: 0, rect: { x: 100, y: 100, width: 500, height: 400 } },
    { kind: 'element', windowIndex: 0, rect: { x: 120, y: 120, width: 200, height: 80 } },
    { kind: 'window', windowIndex: 1, rect: { x: 0, y: 0, width: 800, height: 800 } },
    { kind: 'element', windowIndex: 1, rect: { x: 130, y: 130, width: 10, height: 10 } },
  ]
  it('优先可见窗口的最小控件，Alt 模式只选父窗口', () => {
    expect(targetAt(targets, { x: 135, y: 135 })).toBe(1)
    expect(targetAt(targets, { x: 135, y: 135 }, true)).toBe(0)
    expect(targetAt(targets, { x: 900, y: 900 })).toBeNull()
  })
  it('隐藏应用可列为录屏来源，但不能挡住可见窗口的截图吸附', () => {
    const hidden: CaptureTarget = { ...targets[0], recordingOnly: true }
    expect(targetAt([hidden, targets[2]], { x: 135, y: 135 }, true)).toBe(1)
    expect(targetAt([hidden], { x: 135, y: 135 })).toBeNull()
  })
  it('跨屏选区在 125% / 150% 视口比例下保持原像素，不按各屏重复缩放', () => {
    const image = { width: 4480, height: 1440 }
    const rect = { x: 1800, y: 60, width: 800, height: 900 }
    for (const scale of [1, 1.25, 1.5, 2]) {
      const viewport = { width: image.width / scale, height: image.height / scale }
      expect(toPixels(fromPixels(rect, image, viewport), viewport, image)).toEqual(rect)
    }
  })
  it('负坐标屏幕定位及空洞回退仍落在真实屏幕内', () => {
    const displays = [{ x: -1920, y: -1080, width: 1920, height: 1080, primary: false, scale: 1 },
      { x: 0, y: 0, width: 2560, height: 1440, primary: true, scale: 1.5 }]
    const image = { width: 4480, height: 2520 }
    expect(displayBounds(displays, { x: -1920, y: -1080 }, { x: 50, y: 50 }, image, image)).toMatchObject({ x: 0, y: 0, width: 1920 })
    expect(displayBounds(displays, { x: -1920, y: -1080 }, { x: 2000, y: 50 }, image, image)).toMatchObject({ x: 1920, y: 1080, width: 2560 })
  })
  it('只在阈值内吸附，不影响自由框选远端像素', () => {
    expect(snapPoint({ x: 103, y: 200 }, targets[0].rect, { x: 6, y: 6 })).toEqual({ x: 100, y: 200 })
    expect(snapPoint({ x: 107, y: 200 }, targets[0].rect, { x: 6, y: 6 })).toEqual({ x: 107, y: 200 })
  })
  it('桌面尺寸相同但屏幕排列变化时，不恢复旧选区', () => {
    const image = { width: 4480, height: 1440, layoutKey: 'left-primary' }
    const recent = parseRecentSelection(JSON.stringify({ version: 1, image, rect: targets[0].rect }))
    expect(restoreRecentSelection(recent, image, image)).toEqual(targets[0].rect)
    expect(restoreRecentSelection(recent, { ...image, layoutKey: 'right-primary' }, image)).toBeNull()
  })
  it('光标默认隐藏，吸附默认开启', () => {
    expect(defaultSettings()).toMatchObject({ screenshotCursor: false, screenshotSnap: true })
  })
})
