import { createApp } from 'vue'
import RecordingControls from './components/RecordingControls.vue'
import './recording-controls.css'

// 原生控制窗直接加载此入口，避免主界面和截图编辑器的加载影响录屏按钮。
createApp(RecordingControls).mount('#app')
