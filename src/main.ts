import { createApp, defineComponent, h } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { resolveWindowKind } from './lib/window-kind'

// 原生窗口标签是固定契约，避免辅助窗口导航/重载后查询参数丢失挂错页面。
async function mount() {
  const kind = resolveWindowKind(isTauri() ? getCurrentWindow().label : null, location.search)
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
  if (kind === 'selection-edge' || kind === 'recording-controls') {
    // 窄原生窗口无法容纳窗口外浮层，控制条使用系统 title 提示。
    createApp(component).mount('#app')
  } else {
    const TooltipLayer = (await import('./components/TooltipLayer.vue')).default
    createApp(defineComponent({ render: () => [h(component), h(TooltipLayer)] })).mount('#app')
  }
}
void mount().catch(error => {
  const root = document.getElementById('app')
  if (root) root.textContent = `页面加载失败，请重启 Snap Desk：${String(error)}`
})
