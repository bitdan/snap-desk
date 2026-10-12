import { createApp } from 'vue'
import App from './App.vue'
import ScrollPreview from './components/ScrollPreview.vue'
import CaptureOverlay from './components/CaptureOverlay.vue'
import PinWindow from './components/PinWindow.vue'
import RecordingControls from './components/RecordingControls.vue'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import './style.css'

// 原生窗口标签是固定契约，避免辅助窗口导航/重载后查询参数丢失挂错页面。
const nativeLabel = isTauri() ? getCurrentWindow().label : null
const windowKind = nativeLabel === 'recording-controls' ? nativeLabel : new URLSearchParams(location.search).get('window')
const overlay = windowKind === 'capture'
document.documentElement.classList.toggle('capture-window', overlay)
document.documentElement.classList.toggle('pin-window', windowKind === 'pin')
createApp(windowKind === 'recording-controls' ? RecordingControls : windowKind === 'scroll-preview' || windowKind === 'scroll-controls' ? ScrollPreview : overlay ? CaptureOverlay : windowKind === 'pin' ? PinWindow : App).mount('#app')
