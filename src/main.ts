import { createApp } from 'vue'
import App from './App.vue'
import CaptureOverlay from './components/CaptureOverlay.vue'
import './style.css'

const overlay = new URLSearchParams(location.search).get('window') === 'capture'
document.documentElement.classList.toggle('capture-window', overlay)
createApp(overlay ? CaptureOverlay : App).mount('#app')
