import { initializeAppearance, applyAppearance, parseAppearance } from './lib/appearance'
import { t } from './lib/i18n'
import { listen } from '@tauri-apps/api/event'
import { createApp, defineComponent, h, nextTick } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { resolveWindowKind } from './lib/window-kind'
import { appLog, installGlobalErrorLogging } from './lib/logger'
import { api } from './lib/api'

initializeAppearance()
installGlobalErrorLogging(window)

// 原生窗口标签是固定契约，避免辅助窗口导航/重载后查询参数丢失挂错页面。
async function mount() {
  const kind = resolveWindowKind(isTauri() ? getCurrentWindow().label : null, location.search)
  // 原生设置提交后广播外观，已打开的辅助 WebView 也立即切换；缓存仅负责首帧。
  if (isTauri() && kind !== 'selection-edge') void listen('appearance-changed', event => {
    const value = parseAppearance(event.payload)
    if (value) applyAppearance(value)
  }).catch(() => appLog('error', 'frontend.appearance_listener_failed'))
  document.documentElement.classList.toggle('capture-window', kind === 'capture')
  document.documentElement.classList.toggle('pin-window', kind === 'pin')
  document.documentElement.classList.toggle('selection-edge', kind === 'selection-edge')
  let component
  if (kind === 'selection-edge') {
    component = (await import('./components/SelectionEdge.vue')).default
  } else if (kind === 'recording-controls') {
    await import('./recording-controls.css')
    component = (await import('./components/RecordingControls.vue')).default
  } else {
    await import('./style.css')
    component = kind === 'capture' ? (await import('./components/CaptureOverlay.vue')).default
      : kind === 'pin' ? (await import('./components/PinWindow.vue')).default
      : kind === 'scroll-preview' || kind === 'scroll-controls' ? (await import('./components/ScrollPreview.vue')).default
      : (await import('./App.vue')).default
  }
  if (kind === 'selection-edge' || kind === 'recording-controls' || kind === 'scroll-controls') {
    // 窄原生窗口无法容纳窗口外浮层，控制条使用系统 title 提示。
    const app = createApp(component)
    app.config.errorHandler = () => appLog('error', 'vue.component_error')
    app.mount('#app')
  } else {
    const TooltipLayer = (await import('./components/TooltipLayer.vue')).default
    const app = createApp(defineComponent({ render: () => [h(component), h(TooltipLayer)] }))
    app.config.errorHandler = () => appLog('error', 'vue.component_error')
    app.mount('#app')
  }
  if (isTauri() && kind === 'main') {
    // DOM 和样式先落地，再允许原生窗口显示；设置查询在组件内继续异步执行。
    await nextTick()
    await api.mainWindowReady()
  }
}
void mount().catch(error => {
  appLog('error', 'frontend.mount_failed')
  const root = document.getElementById('app')
  if (root) root.textContent = t('页面加载失败，请重启 Snap Desk：{0}', { 0: String(error) })
  // 加载失败也展示错误文本，避免窗口一直隐藏而无法获知原因。
  if (isTauri() && getCurrentWindow().label === 'main') void api.mainWindowReady().catch(() => {})
})
