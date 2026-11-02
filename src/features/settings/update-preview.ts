import type { AppUpdateView } from './types'
import { version } from '../../../package.json'

/** 仅开发预览启用；不请求安装包，也不调用原生安装或退出命令。 */
export const updatePreview = import.meta.env.DEV && import.meta.env.VITE_SIMULATE_UPDATE === 'true'
const listeners = new Set<(view: AppUpdateView) => void>()
const state: AppUpdateView = {
  currentVersion: version, availableVersion: '0.2.1', notes: null,
  packageKind: 'standalone', supported: true, configured: true,
  phase: 'available', downloaded: 0, total: null, error: null,
}
const snapshot = () => ({ ...state })
function publish() { listeners.forEach(fn => fn(snapshot())); return snapshot() }
const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

export function onPreviewUpdate(fn: (view: AppUpdateView) => void) {
  listeners.add(fn)
  return () => { listeners.delete(fn) }
}
export const previewUpdater = {
  status: async () => snapshot(),
  async check() {
    if (['checking', 'downloading', 'installing'].includes(state.phase)) return snapshot()
    state.phase = 'checking'; state.availableVersion = null; publish()
    await wait(600)
    state.phase = 'available'; state.availableVersion = '0.2.1'
    state.downloaded = 0; state.total = null; state.error = null
    return publish()
  },
  async download() {
    if (!['available', 'error'].includes(state.phase)) return snapshot()
    state.phase = 'downloading'; state.downloaded = 0; state.total = 40 * 1048576; publish()
    for (let step = 1; step <= 20; step++) {
      await wait(200)
      state.downloaded = state.total! * step / 20; publish()
    }
    state.phase = 'ready'
    return publish()
  },
  async install() {
    if (state.phase !== 'ready') return
    state.phase = 'installing'; publish()
    await wait(800)
    state.currentVersion = state.availableVersion!; state.availableVersion = null
    state.phase = 'latest'; state.downloaded = 0; state.total = null
    publish()
  },
}
