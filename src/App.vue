<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Camera, Keyboard, Info, Scan, PanelLeftClose, Pin, Video, Folder, CircleHelp, Settings as SettingsIcon, FileText, Check } from 'lucide-vue-next'
import DiagnosticsSettings from '@/components/DiagnosticsSettings.vue'
import GeneralSettings from '@/components/GeneralSettings.vue'
import AiProviderSettings from '@/components/AiProviderSettings.vue'
import OnboardingGuide from '@/components/OnboardingGuide.vue'
import ShortcutInput from '@/components/ShortcutInput.vue'
import RecordingPanel from '@/components/RecordingPanel.vue'
import ClipboardHistory from '@/components/clipboard/ClipboardHistory.vue'
import PinManager from '@/components/PinManager.vue'
import Button from '@/components/ui/button/Button.vue'
import { api, desktop, onDelay, onError, onSettingsChanged, onClipboardOpen, type AppStatus, type Settings } from '@/lib/api'
import { defaultSettings, exportFilenamePreview, validateExportSettings, validateShortcuts, shortcutActions, needsOnboarding } from '@/lib/settings'
import { errorMessage } from '@/lib/errors'

const page = ref('截图')
// 导航按操作、配置与支持分组，日志拥有独立页面。
const groups = [
  { name: '工具', items: [{ name: '截图', icon: Camera }, { name: '录屏', icon: Video }, { name: '贴图', icon: Pin }, { name: '剪贴板', icon: Folder }] },
  { name: '设置', items: [{ name: '通用', icon: SettingsIcon }, { name: '快捷键', icon: Keyboard }, { name: '识别与翻译', icon: Scan }] },
  { name: '支持', items: [{ name: '日志', icon: FileText }, { name: '帮助', icon: CircleHelp }, { name: '关于', icon: Info }] },
]
const status = ref<AppStatus | null>(null)
const pending = ref(false)
const saving = ref(false)
const editingShortcut = ref(false)
const onboarding = ref(false)
const completingOnboarding = ref(false)
const remaining = ref(0)
// 表单草稿与已生效配置分开：保存失败时快捷键提示仍显示原来的值。
const draft = ref<Settings>(defaultSettings())
const savedSection = ref('')
const shortcutExampleCopied = ref(false)
const error = ref('')
const disposers: (() => void)[] = []
let disposed = false
let delayRevision = 0
const initializing = ref(false)
const initializationFailed = ref(false)
const shortcut = computed(() => status.value?.settings.shortcuts.capture || (status.value ? '未设置' : desktop ? '未读取' : 'F1'))
const locked = computed(() => !desktop || !status.value || initializing.value || initializationFailed.value || pending.value || saving.value || editingShortcut.value || remaining.value > 0)
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
    const invalid = validateExportSettings(value.export) ?? validateShortcuts(value.shortcuts)
    if (invalid) throw new Error(invalid)
    const saved = await api.saveSettings(value)
    // 每个入口仅同步自己编辑的字段，避免覆盖另一页尚未保存的草稿。
    if (section === '快捷键') draft.value.shortcuts = { ...saved.shortcuts }
    if (section === '截图') { draft.value.export = { ...saved.export }; draft.value.delaySeconds = saved.delaySeconds; draft.value.screenshotCursor = saved.screenshotCursor; draft.value.screenshotSnap = saved.screenshotSnap }
    await refreshStatus()
    savedSection.value = section
  } catch (e) { error.value = String(e) }
  finally { saving.value = false }
}
/** 保存时以最新已生效设置为底，仅替换当前模块拥有的字段。 */
function saveCapture(reset = false) {
  if (!status.value) return
  const value = reset ? defaultSettings() : draft.value
  void saveSettings({ ...status.value.settings, export: { ...value.export }, screenshotCursor: value.screenshotCursor, screenshotSnap: value.screenshotSnap, delaySeconds: reset ? value.delaySeconds : status.value.settings.delaySeconds }, '截图')
}
function saveShortcut(reset = false) {
  if (!status.value) return
  void saveSettings({ ...status.value.settings, shortcuts: { ...(reset ? defaultSettings().shortcuts : draft.value.shortcuts) } }, '快捷键')
}
async function copyShortcutExample() {
  try { await api.copyShortcutExample(); shortcutExampleCopied.value = true }
  catch (e) { error.value = String(e) }
}
async function openProject() {
  try { await api.openProjectHomepage() } catch (e) { error.value = String(e) }
}
/** 总开关单独提交，不覆盖尚未保存的格式、延时等草稿。 */
async function toggleShortcuts() {
  if (!status.value || saving.value || editingShortcut.value) return
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
    for (const subscribe of [() => onClipboardOpen(() => { page.value = '剪贴板' }), () => onSettingsChanged(() => { void syncSettings() }), () => onError(e => { error.value = e }), () => onDelay(value => { delayRevision++; remaining.value = value })]) {
      const off = await subscribe()
      if (disposed) { off(); return }
      disposers.push(off)
    }
    await api.setShortcutEditing(false)
    await refreshStatus()
    onboarding.value = !status.value!.configWarning && needsOnboarding(status.value!.settings.onboardingVersion)
    draft.value = { ...status.value!.settings, shortcuts: { ...status.value!.settings.shortcuts }, export: { ...status.value!.settings.export } }
  } catch (e) {
    if (!disposed) {
      initializationFailed.value = true
      error.value = `桌面初始化失败：${errorMessage(e)}`
    }
    for (const off of disposers.splice(0)) off()
  } finally { initializing.value = false }
}
/** 后端合并引导版本，不提交其他页面的未保存草稿。 */
async function completeOnboarding() {
  if (completingOnboarding.value || !status.value) return
  completingOnboarding.value = true
  try {
    const saved = await api.completeOnboarding()
    status.value.settings.onboardingVersion = saved.onboardingVersion
    draft.value.onboardingVersion = saved.onboardingVersion
    onboarding.value = false
  } catch (e) { error.value = String(e) }
  finally { completingOnboarding.value = false }
}
onMounted(initialize)
onUnmounted(() => { disposed = true; disposers.splice(0).forEach(off => off()) })
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand"><span class="brand-icon"><Scan :size="23" /></span><div>Snap Desk</div></div>
      <nav v-for="group in groups" :key="group.name" :aria-label="group.name"><div class="nav-caption">{{ group.name }}</div><button v-for="item in group.items" :key="item.name" :class="['nav-item', { active: page === item.name }]" @click="page = item.name"><component :is="item.icon" :size="18" />{{ item.name }}</button></nav>
      <div class="sidebar-bottom"><span class="version-dot" /> {{ status ? `v${status.version}` : '版本待读取' }}</div>
    </aside>
    <main class="workspace">
      <header class="topbar"><span>{{ page }}</span><div class="flex items-center gap-2"><Button variant="ghost" :disabled="locked" :aria-pressed="status?.settings.shortcutsEnabled" :title="status?.settings.shortcutsEnabled ? '禁用全部全局快捷键' : '启用全部全局快捷键'" @click="toggleShortcuts"><Check v-if="status?.settings.shortcutsEnabled && !status.shortcutError" :size="15" class="status-check" /><Keyboard v-else :size="15" />快捷键{{ !status ? '未读取' : status.shortcutError ? '注册异常' : status.settings.shortcutsEnabled ? '已启用' : '已禁用' }}</Button><Button variant="ghost" title="隐藏主窗口，保留托盘运行" :disabled="!desktop" @click="hide"><PanelLeftClose :size="15" />收起</Button></div></header>
      <div class="page-content" :class="{ 'clipboard-page': page === '剪贴板', 'logs-page': page === '日志' }">
        <div v-if="!desktop" class="notice">当前为网页预览。请通过桌面应用启动以使用屏幕截图和全局快捷键。</div>
        <div v-if="error || status?.shortcutError" class="notice error" role="alert">{{ error || status?.shortcutError }}<Button v-if="initializationFailed" variant="outline" :disabled="initializing" @click="initialize">重新连接</Button></div>
        <div v-if="status?.configWarning" class="notice" role="alert">{{ status.configWarning }}</div>
        <div v-if="remaining > 0" class="notice flex items-center justify-between" role="status"><span>{{ remaining }} 秒后截图，可切换到目标窗口；再次按截图快捷键可取消。</span><Button variant="outline" @click="cancelPending">取消截图</Button></div>

        <OnboardingGuide v-if="onboarding && status" :settings="status.settings" :busy="completingOnboarding" @complete="completeOnboarding" @navigate="page = $event" />
        <template v-if="page === '截图'">
          <section class="capture-panel" aria-label="截图操作">
            <div class="capture-controls">
              <kbd>{{ status?.settings.shortcutsEnabled === false ? '快捷键已禁用' : shortcut }}</kbd>
              <label for="quick-delay">延时</label>
              <!-- 主页使用同一设置保存接口，确保按钮、托盘和快捷键的延时一致。 -->
              <select id="quick-delay" :value="status?.settings.delaySeconds ?? 0" :disabled="locked" @change="changeDelay"><option :value="0">无延时</option><option :value="3">3 秒</option><option :value="5">5 秒</option></select>
              <Button :disabled="locked" @click="start"><Camera :size="16" />{{ remaining > 0 ? `${remaining} 秒后截图` : pending ? '正在准备…' : '开始截图' }}</Button>
            </div>
          </section>

          <h2 class="mt-4 mb-2 text-sm font-semibold">截图设置</h2>
          <form class="settings-card mb-5" @submit.prevent="saveCapture()" @input="savedSection = ''">
            <div class="setting-row"><label for="screenshot-cursor">截图包含光标</label><input id="screenshot-cursor" v-model="draft.screenshotCursor" type="checkbox" :disabled="locked || status?.platform !== 'windows'" /></div>
            <div class="setting-row"><label for="screenshot-snap">窗口 / 元素吸附</label><input id="screenshot-snap" v-model="draft.screenshotSnap" type="checkbox" :disabled="locked || status?.platform !== 'windows'" /></div>
            <div class="setting-row"><label for="export-format">保存格式</label><select id="export-format" v-model="draft.export.format" :disabled="locked" class="rounded-lg border border-zinc-200 bg-white px-3 py-2"><option value="png">PNG · 无损透明</option><option value="jpeg">JPEG · 白底</option><option value="webp">WebP · 无损透明</option></select></div>
            <div v-if="draft.export.format === 'jpeg'" class="setting-row"><label for="jpeg-quality">JPEG 质量</label><input id="jpeg-quality" v-model.number="draft.export.jpegQuality" type="number" min="1" max="100" step="1" required :disabled="locked" class="w-24 rounded-lg border border-zinc-200 px-3 py-2" /></div>
            <div class="setting-row"><label for="filename-prefix">文件名前缀</label><input id="filename-prefix" v-model="draft.export.filenamePrefix" required :disabled="locked" class="w-48 rounded-lg border border-zinc-200 px-3 py-2 text-sm" /></div>
            <div class="setting-row"><label for="filename-dimensions">文件名包含图片尺寸<small class="break-all">示例：{{ exportFilenamePreview(draft.export) }}</small></label><input id="filename-dimensions" v-model="draft.export.includeDimensions" type="checkbox" :disabled="locked" /></div>
            <div class="setting-row"><label for="remember-directory">记住上次保存目录</label><input id="remember-directory" v-model="draft.export.rememberDirectory" type="checkbox" :disabled="locked" /></div>
            <div class="flex gap-2 py-3"><Button type="submit" :disabled="locked">{{ saving ? '正在保存…' : '保存设置' }}</Button><Button type="button" variant="outline" :disabled="locked" @click="saveCapture(true)">恢复默认</Button><span v-if="savedSection === '截图'" class="status-ok self-center" role="status"><Check :size="14" />已保存</span></div>
          </form>
        </template>

        <ClipboardHistory v-else-if="page === '剪贴板'" />
        <DiagnosticsSettings v-else-if="page === '日志'" />
        <GeneralSettings v-else-if="page === '通用'" />
        <AiProviderSettings v-else-if="page === '识别与翻译'" />
        <template v-else-if="page === '帮助'">
          <Button :disabled="!status || completingOnboarding" @click="onboarding = true">重新打开新手引导</Button>
        </template>
        <RecordingPanel v-else-if="page === '录屏'" />
        <PinManager v-else-if="page === '贴图'" />
        <template v-else-if="page === '快捷键'">
          <form class="settings-card" @submit.prevent="saveShortcut()">
            <div v-for="action in shortcutActions" :key="action.key" class="setting-row flex-wrap gap-3">
              <label>{{ action.label }}<small>{{ action.description }}</small></label>
              <div class="flex flex-wrap items-center gap-2">
                <ShortcutInput v-model="draft.shortcuts[action.key]" :label="action.label" :disabled="locked" :validate="value => validateShortcuts({ ...draft.shortcuts, [action.key]: value })" @editing="editingShortcut = $event" @error="error = $event" @update:model-value="savedSection = ''" />
                <Button type="button" variant="ghost" :disabled="locked || !draft.shortcuts[action.key]" @click="draft.shortcuts[action.key] = ''; savedSection = ''">禁用</Button>
                <Button type="button" variant="ghost" :disabled="locked" @click="draft.shortcuts[action.key] = defaultSettings().shortcuts[action.key]; savedSection = ''">默认</Button>
              </div>
            </div>
            <p class="py-3 text-xs text-zinc-500">点击快捷键框后直接按键。F1～F12 可单独使用，字母和数字须搭配修饰键；清空即禁用。修改后点击保存生效。</p>
            <div class="flex items-center gap-2 py-3"><Button type="submit" :disabled="locked">{{ saving ? '正在保存…' : '保存快捷键' }}</Button><Button type="button" variant="outline" :disabled="locked" @click="saveShortcut(true)">恢复默认</Button><Button type="button" variant="ghost" :disabled="locked" @click="copyShortcutExample">{{ shortcutExampleCopied ? '示例已复制' : '复制示例配置' }}</Button><span v-if="savedSection === '快捷键'" class="status-ok" role="status"><Check :size="14" />已保存</span></div>
          </form>
        </template>

        <template v-else>
          <section class="settings-card about"><span class="brand-icon"><Scan :size="30" /></span><h2>Snap Desk <span class="pill">{{ status?.version ?? '桌面版本待读取' }}</span></h2><p>截图、标注、贴图、长截图与录屏，本地处理。</p><p>Windows 截图支持多屏；录屏与长截图限主屏，录屏需 Windows 10 2004 或更新版本。</p><div class="flex items-center gap-3"><a href="https://github.com/bitdan/snap-desk" target="_blank" rel="noopener noreferrer" class="github-link" aria-label="在浏览器打开 GitHub 项目主页" title="GitHub" @click="desktop && ($event.preventDefault(), openProject())"><svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor" aria-hidden="true"><path d="M12 .75a11.25 11.25 0 0 0-3.56 21.92c.56.1.77-.24.77-.54v-2.1c-3.13.68-3.79-1.33-3.79-1.33-.51-1.3-1.25-1.65-1.25-1.65-1.02-.7.08-.69.08-.69 1.13.08 1.73 1.16 1.73 1.16 1 1.72 2.63 1.22 3.27.93.1-.73.39-1.22.71-1.5-2.5-.28-5.13-1.25-5.13-5.56 0-1.23.44-2.23 1.16-3.01-.12-.28-.5-1.43.11-2.98 0 0 .95-.3 3.1 1.15a10.8 10.8 0 0 1 5.64 0c2.15-1.46 3.1-1.15 3.1-1.15.61 1.55.23 2.7.11 2.98.72.78 1.16 1.78 1.16 3.01 0 4.32-2.63 5.28-5.14 5.56.4.35.76 1.03.76 2.09v3.09c0 .3.21.65.77.54A11.25 11.25 0 0 0 12 .75Z" /></svg></a><span class="text-xs text-zinc-500">GPL-3.0-only</span></div></section>
        </template>
      </div>
    </main>
  </div>
</template>
