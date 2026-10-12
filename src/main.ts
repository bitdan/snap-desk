import { createApp } from 'vue'
import App from './App.vue'
import ScrollPreview from './components/ScrollPreview.vue'
import CaptureOverlay from './components/CaptureOverlay.vue'
import PinWindow from './components/PinWindow.vue'
import './style.css'

const windowKind = new URLSearchParams(location.search).get('window')
const overlay = windowKind === 'capture'
document.documentElement.classList.toggle('capture-window', overlay)
document.documentElement.classList.toggle('pin-window', windowKind === 'pin')
createApp(windowKind === 'scroll-preview' || windowKind === 'scroll-controls' ? ScrollPreview : overlay ? CaptureOverlay : windowKind === 'pin' ? PinWindow : App).mount('#app')
