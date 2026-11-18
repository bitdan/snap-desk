import { describe, expect, it } from 'vitest'
import { featureAvailable, pageLimitations, shortcutCommands } from '@/features/settings/desktop-support'
import type { DesktopFeatures, DesktopSupport } from '@/features/settings/types'

const yes = { available: true, reason: null }
const no = (reason: string) => ({ available: false, reason })
function support(session: DesktopSupport['session'], overrides: Partial<DesktopFeatures> = {}): DesktopSupport {
  const features = Object.fromEntries(['screenshot', 'screenshotCursor', 'windowSnap', 'scrollCapture', 'recording', 'recordingEffects', 'globalShortcuts', 'clipboardHistory', 'selectedText', 'directPaste', 'pinOnTop'].map(key => [key, yes])) as unknown as DesktopFeatures
  return {
    session, desktop: '', compositor: '', commandLine: "'/home/u/My Apps/Snap.AppImage'",
    protocols: { kwinScreenshot: false, extImageCopyCapture: false, wlrScreencopy: false, layerShell: false, extDataControl: false, wlrDataControl: false, portalScreenshot: false, portalScreencast: false, portalGlobalShortcuts: false, portalRemoteDesktop: false, xwayland: false },
    features: { ...features, ...overrides },
  }
}

describe('desktop support', () => {
  it('treats a missing report as available so older backends keep working', () => {
    expect(featureAvailable(undefined, 'screenshot')).toBe(true)
    expect(pageLimitations(null, '截图')).toEqual([])
    expect(shortcutCommands(undefined, [{ key: 'capture', label: '区域截图' }])).toEqual([])
  })

  it('lists each unavailable reason on its page once', () => {
    const pending = 'Wayland 会话暂不支持此功能，请在登录界面选择 X11（Xorg）会话后使用。'
    const value = support('wayland', { screenshot: no(pending), scrollCapture: no(pending), windowSnap: no('吸附'), pinOnTop: no('置顶') })
    expect(featureAvailable(value, 'screenshot')).toBe(false)
    expect(pageLimitations(value, '截图')).toEqual([pending, '吸附'])
    expect(pageLimitations(value, '贴图')).toEqual(['置顶'])
    expect(pageLimitations(value, '日志')).toEqual([])
    expect(pageLimitations(support('x11'), '截图')).toEqual([])
  })

  it('offers bindable commands on Wayland or when global shortcuts are unavailable', () => {
    const actions = [{ key: 'capture', label: '区域截图' }, { key: 'record', label: '录屏' }]
    expect(shortcutCommands(support('x11'), actions)).toEqual([])
    expect(shortcutCommands(support('wayland'), actions)).toHaveLength(2)
    expect(shortcutCommands(support('wayland', { globalShortcuts: no('x') }), actions)).toEqual([
      { label: '区域截图', command: "'/home/u/My Apps/Snap.AppImage' --capture" },
      { label: '录屏', command: "'/home/u/My Apps/Snap.AppImage' --record" },
    ])
  })
})
