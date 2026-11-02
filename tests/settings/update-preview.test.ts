import { afterEach, expect, it, vi } from 'vitest'

afterEach(() => { vi.useRealTimers(); vi.unstubAllEnvs(); vi.resetModules() })

it('默认开发不会启用模拟更新', async () => {
  vi.stubEnv('VITE_SIMULATE_UPDATE', '')
  const { updatePreview } = await import('../../src/features/settings/update-preview')
  expect(updatePreview).toBe(false)
})

it('模拟检查、进度和安装完成，不需要原生环境', async () => {
  vi.useFakeTimers(); vi.stubEnv('VITE_SIMULATE_UPDATE', 'true')
  const { updatePreview, previewUpdater, onPreviewUpdate } = await import('../../src/features/settings/update-preview')
  expect(updatePreview).toBe(true)
  const receive = vi.fn(), off = onPreviewUpdate(receive)
  const check = previewUpdater.check()
  expect((await previewUpdater.status()).phase).toBe('checking')
  await vi.advanceTimersByTimeAsync(600); await check
  expect((await previewUpdater.status()).phase).toBe('available')
  const download = previewUpdater.download()
  await vi.advanceTimersByTimeAsync(1000)
  const progress = await previewUpdater.status()
  expect(progress.phase).toBe('downloading')
  expect(progress.downloaded / progress.total!).toBe(0.25)
  await vi.advanceTimersByTimeAsync(3000); await download
  expect((await previewUpdater.status()).phase).toBe('ready')
  const install = previewUpdater.install()
  expect((await previewUpdater.status()).phase).toBe('installing')
  await vi.advanceTimersByTimeAsync(800); await install
  expect(await previewUpdater.status()).toMatchObject({ phase: 'latest', currentVersion: '0.2.1', availableVersion: null })
  off(); receive.mockClear()
  const again = previewUpdater.check()
  await vi.advanceTimersByTimeAsync(600); await again
  expect(receive).not.toHaveBeenCalled()
})

it('生产环境即使设置模拟标志也不会启用', async () => {
  vi.stubEnv('DEV', false); vi.stubEnv('VITE_SIMULATE_UPDATE', 'true')
  const { updatePreview } = await import('../../src/features/settings/update-preview')
  expect(updatePreview).toBe(false)
})
