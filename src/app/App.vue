<script setup lang="ts">
import { handleEscape } from '@/shared/ui/escape'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import SelectControl from '@/shared/ui/SelectControl.vue'
import { updateSettings } from '@/features/settings/settings-write'
import { t, te } from '@/shared/i18n/i18n'
import { computed, defineAsyncComponent, onMounted, onUnmounted, ref } from 'vue'
import { Camera, Keyboard, Info, Scan, PanelLeftClose, Pin, Video, Folder, CircleHelp, Settings as SettingsIcon, FileText, Check } from 'lucide-vue-next'
const DiagnosticsSettings = defineAsyncComponent(() => import('@/features/diagnostics/components/DiagnosticsSettings.vue'))
const GeneralSettings = defineAsyncComponent(() => import('@/features/settings/components/GeneralSettings.vue'))
const AiProviderSettings = defineAsyncComponent(() => import('@/features/recognition/components/AiProviderSettings.vue'))
import SupportPages from '@/app/components/SupportPages.vue'
import './styles/workspace.css'
import OnboardingGuide from '@/app/components/OnboardingGuide.vue'
import ShortcutInput from '@/features/settings/components/ShortcutInput.vue'
import ShortcutCompatibility from '@/features/settings/components/ShortcutCompatibility.vue'
const RecordingPanel = defineAsyncComponent(() => import('@/features/recording/components/RecordingPanel.vue'))
const ClipboardHistory = defineAsyncComponent(() => import('@/features/clipboard/components/ClipboardHistory.vue'))
const PinManager = defineAsyncComponent(() => import('@/features/pins/components/PinManager.vue'))
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop, onDelay, onError, onSettingsChanged, onClipboardOpen, type Settings } from '@/services/api'
import type { AppStatus } from '@/features/settings/types'
import { defaultSettings, exportFilenamePreview, validateExportSettings, validateShortcuts, shortcutActions, needsOnboarding } from '@/features/settings/settings'
import { errorMessage } from '@/services/errors'
import AppearanceSettings from '@/features/settings/components/AppearanceSettings.vue'
import { applyAppearance, type Appearance } from '@/shared/ui/appearance'

const page = ref('截图')
// 导航按操作、配置与支持分组，日志拥有独立页面。
const groups = [
  { name: '工具', items: [{ name: '截图', icon: Camera }, { name: '录屏', icon: Video }, { name: '贴图', icon: Pin }, { name: '剪贴板', icon: Folder }] },
  { name: '设置', items: [{ name: '通用', icon: SettingsIcon }, { name: '快捷键', icon: Keyboard }, { name: '识别与翻译', icon: Scan }] },
  { name: '支持', items: [{ name: '日志', icon: FileText }, { name: '帮助', icon: CircleHelp }, { name: '关于', icon: Info }] },
]
const pageGroup = computed(() => groups.find(group => group.items.some(item => item.name === page.value))?.name ?? '')
const status = ref<AppStatus | null>(null)
const pending = ref(false)
const saving = ref(false)
const editingShortcut = ref(false)
const testingShortcuts = ref(false)
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
const locked = computed(() => !desktop || !status.value || initializing.value || initializationFailed.value || pending.value || saving.value || editingShortcut.value || testingShortcuts.value || remaining.value > 0)
async function refreshStatus() {
  const revision = delayRevision
  status.value = await api.status()
  applyAppearance(status.value.settings.appearance)
  // 查询返回前若已收到更新事件，不能用旧快照覆盖倒计时结束通知。
  if (revision === delayRevision) remaining.value = status.value.delayRemaining
}
async function saveSettings(value: Settings, section = '') {
  if (locked.value) return
  saving.value = true; error.value = ''; savedSection.value = ''
  try {
    const invalid = validateExportSettings(value.export) ?? validateShortcuts(value.shortcuts)
    if (invalid) throw new Error(invalid)
    await updateSettings(current => section === '通用' ? { ...current, appearance: value.appearance } : { ...current, delaySeconds: value.delaySeconds })
    await refreshStatus()
    savedSection.value = section
  } catch (e) { error.value = String(e) }
  finally { saving.value = false }
}
/** 保存时以最新已生效设置为底，仅替换当前模块拥有的字段。 */
function saveAppearance(value: Appearance) {
  if (status.value) void saveSettings({ ...status.value.settings, appearance: value }, '通用')
}
const captureAutoSave = useAutoSave(async () => {
  const value = { export: { ...draft.value.export }, screenshotCursor: draft.value.screenshotCursor, screenshotSnap: draft.value.screenshotSnap }
  await updateSettings(current => ({ ...current, ...value }))
  await refreshStatus()
}, () => validateExportSettings(draft.value.export))
const shortcutAutoSave = useAutoSave(async () => {
  const shortcuts = { ...draft.value.shortcuts }
  await updateSettings(current => ({ ...current, shortcuts }))
  await refreshStatus()
}, () => editingShortcut.value ? '请先结束快捷键录入或当前截图。' : validateShortcuts(draft.value.shortcuts))
function resetCapture() {
  const value = defaultSettings()
  draft.value.export = value.export; draft.value.screenshotCursor = value.screenshotCursor; draft.value.screenshotSnap = value.screenshotSnap
  captureAutoSave.schedule(); void captureAutoSave.flush()
}
function resetShortcuts() {
  draft.value.shortcuts = defaultSettings().shortcuts
  shortcutAutoSave.schedule(); void shortcutAutoSave.flush()
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
async function changeDelay(value: number) {
  if ((value === 0 || value === 3 || value === 5) && status.value) await saveSettings({ ...status.value.settings, delaySeconds: value })
}
async function start() {
  if (pending.value) return
  pending.value = true; error.value = ''
  try {
    await captureAutoSave.flush()
    if (!captureAutoSave.error.value) await api.start()
  } catch (e) { error.value = String(e) }
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
// 表单输入框同样使用 Esc 收起；弹窗、快捷键录入等内层交互可先消费该事件。
function key(event: KeyboardEvent) { handleEscape(event, () => { if (desktop) void hide() }) }
onMounted(() => { window.addEventListener('keydown', key); void initialize() })
onUnmounted(() => { disposed = true; window.removeEventListener('keydown', key); disposers.splice(0).forEach(off => off()) })
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand"><span class="brand-icon"><Scan :size="23" /></span><div>Snap Desk</div></div>
      <nav v-for="group in groups" :key="group.name" :aria-label="t(group.name)"><div class="nav-caption">{{ t(group.name) }}</div><button v-for="item in group.items" :key="item.name" :class="['nav-item', { active: page === item.name }]" :aria-current="page === item.name ? 'page' : undefined" @click="page = item.name"><component :is="item.icon" :size="18" />{{ t(item.name) }}</button></nav>
      <div class="sidebar-bottom"><span v-if="status" class="version-dot" /> {{ status ? `v${status.version}` : t('版本待读取') }}</div>
    </aside>
    <main class="workspace">
      <header class="topbar"><div class="page-breadcrumb"><span>{{ t(pageGroup) }}</span><span aria-hidden="true">/</span><h1>{{ t(page) }}</h1></div><div class="flex items-center gap-2"><Button variant="ghost" :disabled="locked" :aria-pressed="status?.settings.shortcutsEnabled" :title="t(status?.settings.shortcutsEnabled ? '禁用全部全局快捷键' : '启用全部全局快捷键')" @click="toggleShortcuts"><Check v-if="status?.settings.shortcutsEnabled && !status.shortcutError" :size="15" class="status-check" /><Keyboard v-else :size="15" />{{ t('快捷键') }} {{ !status ? t('未读取') : status.shortcutError ? t('注册异常') : status.settings.shortcutsEnabled ? t('已启用') : t('已禁用') }}</Button><Button variant="ghost" :title="t('隐藏主窗口，保留托盘运行')" :disabled="!desktop" @click="hide"><PanelLeftClose :size="15" />{{ t('收起') }}</Button></div></header>
      <div class="page-content" :data-page="page" :class="{ 'clipboard-page': page === '剪贴板', 'logs-page': page === '日志' }">
        <div v-if="initializing" class="notice" role="status">{{ t("正在读取应用设置…") }}</div>
        <div v-if="!desktop" class="notice">{{ t('请在桌面应用中使用截图和快捷键。') }}</div>
        <div v-if="error || status?.shortcutError" class="notice error" role="alert">{{ te(error || status?.shortcutError) }}<Button v-if="initializationFailed" variant="outline" :disabled="initializing" @click="initialize">{{ t('重新连接') }}</Button></div>
        <div v-if="status?.configWarning" class="notice" role="alert">{{ te(status.configWarning) }}</div>
        <div v-if="remaining > 0" class="notice flex items-center justify-between" role="status"><span>{{ remaining }} {{ t('秒后截图，可切换到目标窗口；再次按截图快捷键可取消。') }}</span><Button variant="outline" @click="cancelPending">{{ t('取消截图') }}</Button></div>

        <OnboardingGuide v-if="onboarding && status" :settings="status.settings" :busy="completingOnboarding" @complete="completeOnboarding" @navigate="page = $event" />
        <template v-if="page === '截图'">
          <section class="capture-panel" :aria-label='t("截图操作")'>
            <div class="capture-controls"><kbd v-if="status?.settings.shortcutsEnabled && status.settings.shortcuts.capture">{{ status.settings.shortcuts.capture }}</kbd>
              <label for="quick-delay">{{ t('延时') }}</label>
              <!-- 主页使用同一设置保存接口，确保按钮、托盘和快捷键的延时一致。 -->
              <SelectControl id="quick-delay" :model-value="status?.settings.delaySeconds ?? 0" :options="[{ value: 0, label: t('无延时') }, { value: 3, label: t('3 秒') }, { value: 5, label: t('5 秒') }]" :disabled="locked" @update:model-value="changeDelay" />
              <Button :disabled="locked" @click="start"><Camera :size="16" />{{ remaining > 0 ? `${remaining} ${t('秒后截图')}` : pending ? t('正在准备…') : t('开始截图') }}</Button>
            </div>
          </section>

          <h2 class="section-heading">{{ t('截图设置') }}</h2>
          <form class="settings-card mb-5" @submit.prevent @input="captureAutoSave.schedule" @change="captureAutoSave.commit" @focusout="captureAutoSave.flush">
            <div class="setting-row"><label for="screenshot-cursor">{{ t('截图包含光标') }}</label><input id="screenshot-cursor" v-model="draft.screenshotCursor" type="checkbox" :disabled="locked || status?.platform !== 'windows'" /></div>
            <div class="setting-row"><label for="screenshot-snap">{{ t('窗口 / 元素吸附') }}</label><input id="screenshot-snap" v-model="draft.screenshotSnap" type="checkbox" :disabled="locked || status?.platform !== 'windows'" /></div>
            <h2 class="section-heading">{{ t("文件与导出") }}</h2>
            <div class="setting-row"><label for="export-format">{{ t('保存格式') }}</label><SelectControl id="export-format" v-model="draft.export.format" :disabled="locked" :options="[{ value: 'png', label: t('PNG · 无损透明') }, { value: 'jpeg', label: t('JPEG · 白底') }, { value: 'webp', label: t('WebP · 无损透明') }]" @update:model-value="captureAutoSave.commit" /></div>
            <div v-if="draft.export.format === 'jpeg'" class="setting-row"><label for="jpeg-quality">{{ t('JPEG 质量') }}</label><input id="jpeg-quality" v-model.number="draft.export.jpegQuality" type="number" min="1" max="100" step="1" required :disabled="locked" class="w-24 rounded-lg border border-zinc-200 px-3 py-2" /></div>
            <div class="setting-row"><label for="filename-prefix">{{ t('文件名前缀') }}</label><input id="filename-prefix" v-model="draft.export.filenamePrefix" required :disabled="locked" class="w-48 rounded-lg border border-zinc-200 px-3 py-2 text-sm" /></div>
            <div class="setting-row"><label for="filename-dimensions">{{ t('文件名包含图片尺寸') }}<small class="break-all">{{ t("示例：") }}{{ t(exportFilenamePreview(draft.export)) }}</small></label><input id="filename-dimensions" v-model="draft.export.includeDimensions" type="checkbox" :disabled="locked" /></div>
            <div class="setting-row"><label for="remember-directory">{{ t('记住上次保存目录') }}</label><input id="remember-directory" v-model="draft.export.rememberDirectory" type="checkbox" :disabled="locked" /></div>
            <div class="flex flex-wrap items-center gap-2 py-3"><Button type="button" variant="outline" :disabled="locked" @click="resetCapture">{{ t('恢复默认') }}</Button><span class="settings-save-status" role="status">{{ captureAutoSave.saving.value ? t('正在保存…') : captureAutoSave.saved.value ? t('已自动保存') : t('更改自动保存') }}</span></div>
            <p v-if="captureAutoSave.error.value" class="notice error" role="alert">{{ te(captureAutoSave.error.value) }} <Button type="button" variant="ghost" @click="captureAutoSave.retry">{{ t('重试') }}</Button></p>
          </form>
        </template>

        <ClipboardHistory v-else-if="page === '剪贴板'" />
        <DiagnosticsSettings v-else-if="page === '日志'" />
        <GeneralSettings v-else-if="page === '通用'">
          <AppearanceSettings :busy="locked" :saved="savedSection === '通用'" @save="saveAppearance" />
        </GeneralSettings>
        <AiProviderSettings v-else-if="page === '识别与翻译'" />
        <SupportPages v-else-if="page === '帮助' || page === '关于'" :page="page" :version="status?.version" :shortcuts="status?.settings.shortcuts" :busy="!status || completingOnboarding" @navigate="page = $event" @onboarding="onboarding = true" @project="openProject" />
        <RecordingPanel v-else-if="page === '录屏'" />
        <PinManager v-else-if="page === '贴图'" />
        <template v-else-if="page === '快捷键'">
          <form class="settings-card" @submit.prevent>
            <div v-for="action in shortcutActions" :key="action.key" class="setting-row flex-wrap gap-3">
              <label>{{ t(action.label) }}<small>{{ t(action.description) }}</small></label>
              <div class="flex flex-wrap items-center gap-2">
                <ShortcutInput v-model="draft.shortcuts[action.key]" :label="action.label" :disabled="locked" :validate="value => validateShortcuts({ ...draft.shortcuts, [action.key]: value })" @editing="editingShortcut = $event; if (!$event) shortcutAutoSave.flush()" @error="error = $event" @update:model-value="shortcutAutoSave.schedule" />
                <Button type="button" variant="ghost" :disabled="locked || !draft.shortcuts[action.key]" @click="draft.shortcuts[action.key] = ''; shortcutAutoSave.commit()">{{ t('禁用') }}</Button>
                <Button type="button" variant="ghost" :disabled="locked" @click="draft.shortcuts[action.key] = defaultSettings().shortcuts[action.key]; shortcutAutoSave.commit()">{{ t('默认') }}</Button>
              </div>
            </div>
            <p class="py-3 text-xs text-zinc-500">{{ t('按键录入快捷键，清空可禁用；修改后自动生效。') }}</p>
            <div class="flex flex-wrap items-center gap-2 py-3"><Button type="button" variant="outline" :disabled="locked" @click="resetShortcuts">{{ t('恢复默认') }}</Button><Button type="button" variant="ghost" :disabled="locked" @click="copyShortcutExample">{{ shortcutExampleCopied ? t('示例已复制') : t('复制示例配置') }}</Button><span class="settings-save-status" role="status">{{ shortcutAutoSave.saving.value ? t('正在保存…') : shortcutAutoSave.saved.value ? t('已自动保存') : t('更改自动保存') }}</span></div>
            <p v-if="shortcutAutoSave.error.value" class="notice error" role="alert">{{ te(shortcutAutoSave.error.value) }} <Button type="button" variant="ghost" @click="shortcutAutoSave.retry">{{ t('重试') }}</Button></p>
          </form>
        </template>

        <ShortcutCompatibility v-show="page === '快捷键'" :disabled="locked" @testing="testingShortcuts = $event" />
      </div>
    </main>
  </div>
</template>
