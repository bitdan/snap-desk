<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Camera, Keyboard, Info, Scan, PanelLeftClose, Pin, Video } from 'lucide-vue-next'
import RecordingPanel from '@/components/RecordingPanel.vue'
import PinManager from '@/components/PinManager.vue'
import Button from '@/components/ui/button/Button.vue'
import { api, desktop, onDelay, onError, onSettingsChanged, type AppStatus, type Settings } from '@/lib/api'
import { defaultSettings, exportFilenamePreview, validateExportSettings } from '@/lib/settings'
import { errorMessage } from '@/lib/errors'

const page = ref('截图')
const items = [{ name: '截图', icon: Camera }, { name: '录屏', icon: Video }, { name: '贴图', icon: Pin }, { name: '快捷键', icon: Keyboard }, { name: '关于', icon: Info }]
const status = ref<AppStatus | null>(null)
const pending = ref(false)
const saving = ref(false)
const remaining = ref(0)
// 表单草稿与已生效配置分开：保存失败时快捷键提示仍显示原来的值。
const draft = ref<Settings>(defaultSettings())
const savedSection = ref('')
const error = ref('')
const disposers: (() => void)[] = []
let disposed = false
let delayRevision = 0
const initializing = ref(false)
const initializationFailed = ref(false)
const shortcut = computed(() => status.value?.settings.shortcut ?? (desktop ? '未读取' : 'F1'))
const locked = computed(() => !desktop || !status.value || initializing.value || initializationFailed.value || pending.value || saving.value || remaining.value > 0)
async function refreshStatus() {
  const revision = delayRevision
  status.value = await api.status()
  // 查询返回前若已收到更新事件，不能用旧快照覆盖倒计时结束通知。
  if (revision === delayRevision) remaining.value = status.value.delayRemaining
}
async function saveSettings(value: Settings, section = '') {
  if (locked.value) return
  saving.value = true; error.value = ''; savedSection.value = ''
  try {
    const invalid = validateExportSettings(value.export)
    if (invalid) throw new Error(invalid)
    const saved = await api.saveSettings({ ...value, shortcut: value.shortcut.trim() })
    // 每个入口仅同步自己编辑的字段，避免覆盖另一页尚未保存的草稿。
    if (section === '快捷键') draft.value.shortcut = saved.shortcut
    if (section === '截图') { draft.value.export = { ...saved.export }; draft.value.delaySeconds = saved.delaySeconds }
    await refreshStatus()
    savedSection.value = section
  } catch (e) { error.value = String(e) }
  finally { saving.value = false }
}
/** 保存时以最新已生效设置为底，仅替换当前模块拥有的字段。 */
function saveCapture(reset = false) {
  if (!status.value) return
  const value = reset ? defaultSettings() : draft.value
  void saveSettings({ ...status.value.settings, export: { ...value.export }, delaySeconds: reset ? value.delaySeconds : status.value.settings.delaySeconds }, '截图')
}
function saveShortcut(reset = false) {
  if (!status.value) return
  void saveSettings({ ...status.value.settings, shortcut: reset ? defaultSettings().shortcut : draft.value.shortcut }, '快捷键')
}
async function openProject() {
  try { await api.openProjectHomepage() } catch (e) { error.value = String(e) }
}
/** 总开关单独提交，不覆盖尚未保存的格式、延时等草稿。 */
async function toggleShortcuts() {
  if (!status.value || saving.value) return
  saving.value = true; error.value = ''
  try {
    const next = await api.setShortcutsEnabled(!status.value.settings.shortcutsEnabled)
    draft.value.shortcutsEnabled = next.shortcutsEnabled
    await refreshStatus()
  } catch (e) { error.value = String(e) }
  finally { saving.value = false }
}
async function syncSettings() {
  try { await refreshStatus(); if (status.value) draft.value.shortcutsEnabled = status.value.settings.shortcutsEnabled }
  catch (e) { error.value = String(e) }
}
async function cancelPending() {
  try { await api.cancelPending() } catch (e) { error.value = String(e) }
}
/** 下拉框产生字符串，先收窄为后端允许的延时值；失败后恢复已生效的显示。 */
async function changeDelay(event: Event) {
  const select = event.target as HTMLSelectElement
  const value = Number(select.value)
  if (value === 0 || value === 3 || value === 5) {
    // 主页仅改延时，必须保留已生效的导出偏好，不使用截图表单未保存的草稿。
    if (status.value) await saveSettings({ ...status.value.settings, delaySeconds: value })
  }
  select.value = String(status.value?.settings.delaySeconds ?? 0)
}
async function start() {
  pending.value = true; error.value = ''
  try { await api.start() } catch (e) { error.value = String(e) }
  finally { pending.value = false }
}
async function hide() {
  try { await api.hide() } catch (e) { error.value = String(e) }
}
/** 初始化失败可以重试；清理本轮已注册的监听器，避免反复重试后重复接收事件。 */
async function initialize() {
  if (!desktop || initializing.value || disposed) return
  initializing.value = true; initializationFailed.value = false; error.value = ''
  try {
    for (const subscribe of [() => onSettingsChanged(() => { void syncSettings() }), () => onError(e => { error.value = e }), () => onDelay(value => { delayRevision++; remaining.value = value })]) {
      const off = await subscribe()
      if (disposed) { off(); return }
      disposers.push(off)
    }
    await refreshStatus()
    draft.value = { ...status.value!.settings, export: { ...status.value!.settings.export } }
  } catch (e) {
    if (!disposed) {
      initializationFailed.value = true
      error.value = `桌面初始化失败：${errorMessage(e)}`
    }
    for (const off of disposers.splice(0)) off()
  } finally { initializing.value = false }
}
onMounted(initialize)
onUnmounted(() => { disposed = true; disposers.splice(0).forEach(off => off()) })
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand"><span class="brand-icon"><Scan :size="23" /></span><div>Snap Desk<small>截图与标注</small></div></div>
      <div class="nav-caption">工具</div>
      <nav><button v-for="item in items" :key="item.name" :class="['nav-item', { active: page === item.name }]" @click="page = item.name"><component :is="item.icon" :size="18" />{{ item.name }}</button></nav>
      <div class="sidebar-bottom"><span class="version-dot" /> v0.1.3<span>主屏截图 · 本地处理</span></div>
    </aside>
    <main class="workspace">
      <header class="topbar"><span>{{ page }}</span><div class="flex items-center gap-2"><Button variant="ghost" :disabled="!desktop || !status || saving" :aria-pressed="status?.settings.shortcutsEnabled" :title="status?.settings.shortcutsEnabled ? '禁用全部全局快捷键' : '启用全部全局快捷键'" @click="toggleShortcuts"><Keyboard :size="15" />快捷键{{ status?.settings.shortcutsEnabled ? '已启用' : '已禁用' }}</Button><Button variant="ghost" title="隐藏主窗口，保留托盘运行" :disabled="!desktop" @click="hide"><PanelLeftClose :size="15" />收起</Button></div></header>
      <div class="page-content">
        <div v-if="!desktop" class="notice">当前为网页预览。请通过桌面应用启动以使用屏幕截图和全局快捷键。</div>
        <div v-if="error || status?.shortcutError" class="notice error" role="alert">{{ error || status?.shortcutError }}<Button v-if="initializationFailed" variant="outline" :disabled="initializing" @click="initialize">重新连接</Button></div>
        <div v-if="status?.configWarning" class="notice" role="alert">{{ status.configWarning }}</div>
        <div v-if="remaining > 0" class="notice flex items-center justify-between" role="status"><span>{{ remaining }} 秒后截图，可切换到目标窗口；再次按截图快捷键可取消。</span><Button variant="outline" @click="cancelPending">取消截图</Button></div>

        <template v-if="page === '截图'">
          <h1>截图</h1><p class="subtitle">选区、标注、复制或保存。</p>
          <section class="capture-panel" aria-label="截图操作">
            <div class="capture-panel-heading"><Scan :size="22" /><div><h2>区域截图</h2><p>启动后拖动鼠标选择范围</p></div><kbd>{{ status?.settings.shortcutsEnabled === false ? '快捷键已禁用' : shortcut }}</kbd></div>
            <div class="capture-controls">
              <label for="quick-delay">延时</label>
              <!-- 主页使用同一设置保存接口，确保按钮、托盘和快捷键的延时一致。 -->
              <select id="quick-delay" :value="status?.settings.delaySeconds ?? 0" :disabled="locked" @change="changeDelay"><option :value="0">无延时</option><option :value="3">3 秒</option><option :value="5">5 秒</option></select>
              <Button :disabled="locked" @click="start"><Camera :size="16" />{{ remaining > 0 ? `${remaining} 秒后截图` : pending ? '正在准备…' : '开始截图' }}</Button>
            </div>
          </section>

          <h2 class="mt-6 mb-3 text-sm font-semibold">截图设置</h2>
          <form class="settings-card mb-5" @submit.prevent="saveCapture()" @input="savedSection = ''">
            <div class="setting-row"><label for="export-format">保存格式</label><select id="export-format" v-model="draft.export.format" :disabled="locked" class="rounded-lg border border-zinc-200 bg-white px-3 py-2"><option value="png">PNG · 无损透明</option><option value="jpeg">JPEG · 白底</option><option value="webp">WebP · 无损透明</option></select></div>
            <div v-if="draft.export.format === 'jpeg'" class="setting-row"><label for="jpeg-quality">JPEG 质量</label><input id="jpeg-quality" v-model.number="draft.export.jpegQuality" type="number" min="1" max="100" step="1" required :disabled="locked" class="w-24 rounded-lg border border-zinc-200 px-3 py-2" /></div>
            <div class="setting-row"><label for="filename-prefix">文件名前缀</label><input id="filename-prefix" v-model="draft.export.filenamePrefix" required :disabled="locked" class="w-48 rounded-lg border border-zinc-200 px-3 py-2 text-sm" /></div>
            <div class="setting-row"><label for="filename-dimensions">文件名包含图片尺寸<small class="break-all">示例：{{ exportFilenamePreview(draft.export) }}</small></label><input id="filename-dimensions" v-model="draft.export.includeDimensions" type="checkbox" :disabled="locked" /></div>
            <div class="setting-row"><label for="remember-directory">记住上次保存目录</label><input id="remember-directory" v-model="draft.export.rememberDirectory" type="checkbox" :disabled="locked" /></div>
            <div class="flex gap-3 py-5"><Button type="submit" :disabled="locked">{{ saving ? '正在保存…' : '保存设置' }}</Button><Button type="button" variant="outline" :disabled="locked" @click="saveCapture(true)">恢复默认</Button><span v-if="savedSection === '截图'" class="self-center text-sm text-zinc-500" role="status">已保存</span></div>
          </form>
        </template>

        <RecordingPanel v-else-if="page === '录屏'" />
        <PinManager v-else-if="page === '贴图'" />
        <template v-else-if="page === '快捷键'">
          <h1>快捷键</h1><p class="subtitle">截图、剪贴板贴图与录屏快捷键共用顶部总开关。</p>
          <form class="settings-card" @submit.prevent="saveShortcut()">
            <div class="setting-row"><label for="capture-shortcut">开始区域截图<small>全局快捷键</small></label><input id="capture-shortcut" v-model="draft.shortcut" required maxlength="48" autocomplete="off" spellcheck="false" :disabled="locked" aria-describedby="shortcut-hint" class="w-48 rounded-lg border border-zinc-200 px-3 py-2 text-sm focus:outline-blue-500" @input="savedSection = ''" /></div>
            <p id="shortcut-hint" class="py-3 text-xs text-zinc-500" >例如 F1、Ctrl+Shift+S、Alt+A。F2、F3 已保留给贴图和录屏。</p>
            <div class="setting-row"><label>剪贴板图片贴图<small>直接读取内存图片，无需保存文件</small></label><kbd>F2</kbd></div>
            <div class="setting-row"><label>录屏<small>框选后设置录屏；录制中再次按下停止并保存</small></label><kbd>F3</kbd></div>
            <div class="flex items-center gap-3 py-5"><Button type="submit" :disabled="locked">{{ saving ? '正在保存…' : '保存快捷键' }}</Button><Button type="button" variant="outline" :disabled="locked" @click="saveShortcut(true)">恢复默认</Button><span v-if="savedSection === '快捷键'" class="text-sm text-zinc-500" role="status">已保存</span></div>
          </form>
        </template>

        <template v-else>
          <h1>关于 Snap Desk</h1><p class="subtitle">桌面截图与标注工具</p>
          <section class="settings-card about"><span class="brand-icon"><Scan :size="30" /></span><h2>Snap Desk <span class="pill">0.1.3</span></h2><p>截图、标注、贴图、长截图与录屏，本地处理。</p><p>当前支持主显示器；录屏需 Windows 10 2004 或更新版本。</p><div class="flex items-center gap-3"><a href="https://github.com/bitdan/snap-desk" target="_blank" rel="noopener noreferrer" class="github-link" aria-label="在浏览器打开 GitHub 项目主页" title="GitHub" @click="desktop && ($event.preventDefault(), openProject())"><svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor" aria-hidden="true"><path d="M12 .75a11.25 11.25 0 0 0-3.56 21.92c.56.1.77-.24.77-.54v-2.1c-3.13.68-3.79-1.33-3.79-1.33-.51-1.3-1.25-1.65-1.25-1.65-1.02-.7.08-.69.08-.69 1.13.08 1.73 1.16 1.73 1.16 1 1.72 2.63 1.22 3.27.93.1-.73.39-1.22.71-1.5-2.5-.28-5.13-1.25-5.13-5.56 0-1.23.44-2.23 1.16-3.01-.12-.28-.5-1.43.11-2.98 0 0 .95-.3 3.1 1.15a10.8 10.8 0 0 1 5.64 0c2.15-1.46 3.1-1.15 3.1-1.15.61 1.55.23 2.7.11 2.98.72.78 1.16 1.78 1.16 3.01 0 4.32-2.63 5.28-5.14 5.56.4.35.76 1.03.76 2.09v3.09c0 .3.21.65.77.54A11.25 11.25 0 0 0 12 .75Z" /></svg></a><span class="text-xs text-zinc-500">GPL-3.0-only</span></div></section>
        </template>
      </div>
    </main>
  </div>
</template>
