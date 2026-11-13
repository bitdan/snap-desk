<script setup lang="ts">
import { handleWindowEscape as handleEscape } from '@/shared/ui/escape'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import SelectControl from '@/shared/ui/SelectControl.vue'
import SettingRow from '@/shared/ui/SettingRow'
import { updateSettings } from '@/features/settings/settings-write'
import { t, te } from '@/shared/i18n/i18n'
import { computed, defineAsyncComponent, onMounted, onUnmounted, ref, watch } from 'vue'
import { Camera, Keyboard, Info, Scan, Languages, Pin, Video, ClipboardList, Settings as SettingsIcon, FileText, Search, X } from 'lucide-vue-next'
import SwitchControl from '@/shared/ui/SwitchControl'
import SegmentedControl from '@/shared/ui/SegmentedControl.vue'
import ToolHero from '@/shared/ui/ToolHero.vue'
const DiagnosticsSettings = defineAsyncComponent(() => import('@/features/diagnostics/components/DiagnosticsSettings.vue'))
const ConfigTransferSettings = defineAsyncComponent(() => import('@/features/settings/components/ConfigTransferSettings.vue'))
const GeneralSettings = defineAsyncComponent(() => import('@/features/settings/components/GeneralSettings.vue'))
const OcrSettings = defineAsyncComponent(() => import('@/features/recognition/components/OcrSettings.vue'))
const ServiceSettings = defineAsyncComponent(() => import('@/features/recognition/components/ServiceSettings.vue'))
const AiProviderSettings = defineAsyncComponent(() => import('@/features/recognition/components/AiProviderSettings.vue'))
import SupportPages from '@/app/components/SupportPages.vue'
import './styles/workspace.css'
import '@/features/settings/settings.css'
import OnboardingGuide from '@/app/components/OnboardingGuide.vue'
import ShortcutInput from '@/features/settings/components/ShortcutInput.vue'
import ShortcutCompatibility from '@/features/settings/components/ShortcutCompatibility.vue'
const loadRecordingPanel = () => import('@/features/recording/components/RecordingPanel.vue')
const RecordingPanel = defineAsyncComponent(loadRecordingPanel)
const ClipboardSettings = defineAsyncComponent(() => import('@/features/clipboard/components/ClipboardSettings.vue'))
const ShelfSettings = defineAsyncComponent(() => import('@/features/shelf/components/ShelfSettings.vue'))
const PinManager = defineAsyncComponent(() => import('@/features/pins/components/PinManager.vue'))
import Button from '@/shared/ui/button/Button.vue'
import { api, desktop, onDelay, onError, onSettingsChanged, onSettingsImported, type Settings } from '@/services/api'
import type { AppStatus } from '@/features/settings/types'
import { defaultSettings, exportFilenamePreview, validateExportSettings, validateShortcuts, shortcutActions, needsOnboarding } from '@/features/settings/settings'
import { errorMessage } from '@/services/errors'
import { logFrontendFailure } from '@/services/logger'
import AppearanceSettings from '@/features/settings/components/AppearanceSettings.vue'
import { applyAppearance, type Appearance } from '@/shared/ui/appearance'
import UpdateNotice from '@/features/settings/components/UpdateNotice.vue'

const availableUpdate = ref<string | null>(null)

const page = ref('截图')
/** 导航悬停或键盘聚焦时只预取模块，不挂载页面，也不提前查询设备或启动轮询。 */
function prefetchPage(name: string) {
  if (name === '录屏') void loadRecordingPanel().catch(() => {})
}
// 导航按操作、配置与支持分组；关键词让搜索也能命中页面内的设置项。
const groups = [
  { name: '工具', items: [
    { name: '截图', icon: Camera, keywords: '区域 延时 光标 吸附 保存 格式 文件名 capture screenshot delay format' },
    { name: '录屏', icon: Video, keywords: 'mp4 gif 帧率 画质 码率 编码 倒计时 键鼠 特效 record video fps' },
    { name: '贴图', icon: Pin, keywords: '阴影 边框 圆角 外观 pin shadow border' },
    { name: '剪贴板', icon: ClipboardList, keywords: '历史 记录 清理 粘贴 排除 暂存架 clipboard history shelf' },
  ] },
  { name: '设置', items: [
    { name: '通用', icon: SettingsIcon, keywords: '外观 主题 深色 语言 开机 自启 字体 目录 导入 导出 配置 备份 theme language startup font directory import export' },
    { name: '快捷键', icon: Keyboard, keywords: '按键 冲突 检测 shortcut hotkey' },
    { name: '识别与翻译', icon: Languages, keywords: 'ocr 文字识别 翻译 服务 模型 语言包 密钥 deepl openai deepseek groq paddle translate service' },
  ] },
  { name: '支持', items: [
    { name: '日志', icon: FileText, keywords: '记录级别 诊断 排错 logs diagnostics' },
    { name: '关于与帮助', icon: Info, keywords: '版本 更新 测试版 引导 帮助 许可 about help update version' },
  ] },
]
type RecognitionTab = 'ocr' | 'translation' | 'services'
const recognitionTab = ref<RecognitionTab>('ocr')
const recognitionTabs: { value: RecognitionTab; label: string }[] = [{ value: 'ocr', label: '文字识别' }, { value: 'translation', label: '翻译' }, { value: 'services', label: '服务管理' }]
// 旧页面名（引导、帮助链接、外部入口）映射到合并后的页面。
const pageAliases: Record<string, { page: string; tab?: RecognitionTab }> = {
  '文字识别': { page: '识别与翻译', tab: 'ocr' }, '翻译': { page: '识别与翻译', tab: 'translation' }, '服务管理': { page: '识别与翻译', tab: 'services' },
  '帮助': { page: '关于与帮助' }, '关于': { page: '关于与帮助' }, '导入/导出': { page: '通用' },
}
function navigate(name: string) {
  const alias = pageAliases[name]
  if (alias?.tab) recognitionTab.value = alias.tab
  page.value = alias?.page ?? name
}
const query = ref('')
const visibleGroups = computed(() => {
  const value = query.value.trim().toLowerCase()
  if (!value) return groups
  return groups.map(group => ({ ...group, items: group.items.filter(item => `${item.name} ${t(item.name)} ${item.keywords}`.toLowerCase().includes(value)) })).filter(group => group.items.length)
})
const status = ref<AppStatus | null>(null)
const pending = ref(false)
const saving = ref(false)
const editingShortcut = ref(false)
const testingShortcuts = ref(false)
const onboarding = ref(false)
const onboardingError = ref('')
const completingOnboarding = ref(false)
const remaining = ref(0)
// 表单草稿与已生效配置分开：保存失败时快捷键提示仍显示原来的值。
const draft = ref<Settings>(defaultSettings())
const savedSection = ref('')
const shortcutExampleCopied = ref(false)
const error = ref('')
const disposers: (() => void)[] = []
let disposed = false
let delayRevision = 0, configRevision = 0, statusRevision = 0
const initializing = ref(false)
const initializationFailed = ref(false)
const locked = computed(() => !desktop || !status.value || initializing.value || initializationFailed.value || pending.value || saving.value || editingShortcut.value || testingShortcuts.value || remaining.value > 0)
async function refreshStatus() {
  const revision = delayRevision, config = configRevision, request = ++statusRevision
  const next = await api.status()
  // 并发查询可能乱序返回；只接受最后一次请求的结果，避免旧快照覆盖新状态。
  if (disposed || config !== configRevision || request !== statusRevision) return
  status.value = next
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
  const value = { export: { ...draft.value.export }, screenshotCursor: draft.value.screenshotCursor, screenshotSnap: draft.value.screenshotSnap, screenshotAfter: draft.value.screenshotAfter }
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
  draft.value.export = value.export; draft.value.screenshotCursor = value.screenshotCursor; draft.value.screenshotSnap = value.screenshotSnap; draft.value.screenshotAfter = value.screenshotAfter
  captureAutoSave.schedule(); void captureAutoSave.flush()
}
function resetShortcuts() {
  draft.value.shortcuts = defaultSettings().shortcuts
  shortcutAutoSave.schedule(); void shortcutAutoSave.flush()
}
/** 配置转移先提交未完成的自动保存，避免导出遗漏或旧草稿覆盖导入。 */
async function prepareConfigTransfer() {
  await captureAutoSave.flush()
  await shortcutAutoSave.flush()
  return !captureAutoSave.error.value && !shortcutAutoSave.error.value
}
function configImported(settings: Settings) {
  configRevision++
  error.value = ''
  onboarding.value = needsOnboarding(settings.onboardingVersion)
  applyAppearance(settings.appearance)
  draft.value = structuredClone(settings)
  if (status.value) status.value.settings = structuredClone(settings)
  void refreshStatus().catch(e => { error.value = String(e) })
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
/** 事件已携带完整配置，直接更新状态；不再为每次保存额外查询一次 app_status。 */
function applySettingsSnapshot(next: Settings) {
  if (status.value) status.value.settings = next
  draft.value.shortcutsEnabled = next.shortcutsEnabled
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
    for (const subscribe of [() => onSettingsChanged(applySettingsSnapshot), () => onSettingsImported(configImported), () => onError(e => { error.value = e }), () => onDelay(value => { delayRevision++; remaining.value = value })]) {
      // 单个事件订阅失败只影响该通知，不能把整个主界面判为初始化失败。
      try {
        const off = await subscribe()
        if (disposed) { off(); return }
        disposers.push(off)
      } catch (e) { logFrontendFailure('frontend.subscription_failed', e) }
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
  completingOnboarding.value = true; onboardingError.value = ''
  try {
    const saved = await api.completeOnboarding()
    status.value.settings.onboardingVersion = saved.onboardingVersion
    draft.value.onboardingVersion = saved.onboardingVersion
    onboarding.value = false
  } catch (e) { onboardingError.value = String(e) }
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
      <div class="brand"><span class="brand-icon"><Scan :size="14" :stroke-width="2.4" /></span>Snap Desk</div>
      <label class="nav-search"><Search :size="14" aria-hidden="true" /><input v-model="query" type="search" :placeholder="t('搜索')" :aria-label="t('搜索设置')" /></label>
      <nav v-for="group in visibleGroups" :key="group.name" :aria-label="t(group.name)"><div class="nav-caption">{{ t(group.name) }}</div><button v-for="item in group.items" :key="item.name" :class="['nav-item', { active: page === item.name }]" :aria-current="page === item.name ? 'page' : undefined" :data-onboarding="`nav-${item.name}`" @pointerenter="prefetchPage(item.name)" @focus="prefetchPage(item.name)" @click="navigate(item.name)"><component :is="item.icon" />{{ t(item.name) }}<span v-if="item.name === '关于与帮助' && availableUpdate" class="nav-update-dot" role="status" :aria-label="t('发现新版本')" :title="t('发现新版本')" /></button></nav>
      <p v-if="!visibleGroups.length" class="nav-empty">{{ t('没有匹配的页面') }}</p>
      <div class="sidebar-bottom"><span v-if="status" class="version-dot" />{{ status ? `v${status.version}` : t('版本待读取') }}</div>
    </aside>
    <main class="workspace">
      <header class="topbar">
        <h1>{{ t(page) }}</h1>
        <div class="topbar-status">
          <span v-if="status?.shortcutError" class="status-error" role="status">{{ t('注册异常') }}</span>
          <label for="shortcuts-enabled">{{ t('全局快捷键') }}</label>
          <SwitchControl id="shortcuts-enabled" class="switch-small" :model-value="status?.settings.shortcutsEnabled === true" :disabled="locked" :title="t(status?.settings.shortcutsEnabled ? '禁用全部全局快捷键' : '启用全部全局快捷键')" @update:model-value="toggleShortcuts" />
        </div>
      </header>
      <div class="page-content" :data-page="page" :class="{ 'logs-page': page === '日志' }">
        <UpdateNotice :show-trigger="page !== '关于与帮助'" @version="availableUpdate = $event" />
        <div v-if="initializing" class="notice" role="status">{{ t("正在读取应用设置…") }}</div>
        <div v-if="!desktop" class="notice">{{ t('请在桌面应用中使用截图和快捷键。') }}</div>
        <div v-if="error || status?.shortcutError" class="notice error" role="alert">{{ te(error || status?.shortcutError) }}<Button v-if="initializationFailed" variant="outline" :disabled="initializing" @click="initialize">{{ t('重新连接') }}</Button></div>
        <div v-if="status?.configWarning" class="notice" role="alert">{{ te(status.configWarning) }}</div>
        <div v-if="remaining > 0" class="notice flex items-center justify-between" role="status"><span>{{ remaining }} {{ t('秒后截图，可切换到目标窗口；再次按截图快捷键可取消。') }}</span><Button variant="outline" @click="cancelPending">{{ t('取消截图') }}</Button></div>

        <OnboardingGuide v-if="onboarding && status" :busy="completingOnboarding" :error="onboardingError" @complete="completeOnboarding" @navigate="navigate" />
        <template v-if="page === '截图'">
          <ToolHero :icon="Camera" :title="t('区域截图')" :shortcut="status?.settings.shortcutsEnabled ? status.settings.shortcuts.capture : ''" data-onboarding="capture-controls">
            <Button :disabled="locked" @click="start">{{ remaining > 0 ? `${remaining} ${t('秒后截图')}` : pending ? t('正在准备…') : t('开始截图') }}</Button>
          </ToolHero>
          <form @submit.prevent @input="captureAutoSave.schedule" @change="captureAutoSave.commit" @focusout="captureAutoSave.flush">
            <h2 class="section-heading">{{ t('截图') }}<button type="button" :disabled="locked" @click="resetCapture">{{ t('恢复默认') }}</button></h2>
            <div class="settings-card">
              <!-- 主页使用同一设置保存接口，确保按钮、托盘和快捷键的延时一致。 -->
              <SettingRow :label="t('延时')"><SegmentedControl :model-value="status?.settings.delaySeconds ?? 0" :label="t('延时')" :options="[{ value: 0, label: t('无延时') }, { value: 3, label: t('3 秒') }, { value: 5, label: t('5 秒') }]" :disabled="locked" @update:model-value="changeDelay" /></SettingRow>
              <SettingRow :label="t('截图包含光标')" for="screenshot-cursor"><SwitchControl id="screenshot-cursor" v-model="draft.screenshotCursor" :disabled="locked || status?.platform !== 'windows'" @change="captureAutoSave.commit" /></SettingRow>
              <SettingRow :label="t('窗口 / 元素吸附')" for="screenshot-snap"><SwitchControl id="screenshot-snap" v-model="draft.screenshotSnap" :disabled="locked || status?.platform !== 'windows'" @change="captureAutoSave.commit" /></SettingRow>
              <SettingRow :label="t('框选完成后')" for="screenshot-after" :hint="draft.screenshotAfter === 'none' ? '' : t('按住 Ctrl 框选可保留工具栏')"><SelectControl id="screenshot-after" v-model="draft.screenshotAfter" :disabled="locked" :options="[{ value: 'none', label: t('显示工具栏') }, { value: 'copy', label: t('直接复制') }, { value: 'pin', label: t('直接贴图') }, { value: 'save', label: t('直接保存') }]" @update:model-value="captureAutoSave.commit" /></SettingRow>
            </div>
            <h2 class="section-heading">{{ t("文件与导出") }}</h2>
            <div class="settings-card">
              <SettingRow :label="t('保存格式')"><SegmentedControl v-model="draft.export.format" :label="t('保存格式')" :disabled="locked" :options="[{ value: 'png', label: 'PNG' }, { value: 'jpeg', label: 'JPEG' }, { value: 'webp', label: 'WebP' }]" @update:model-value="captureAutoSave.commit" /></SettingRow>
              <SettingRow v-if="draft.export.format === 'jpeg'" :label="t('JPEG 质量')" for="jpeg-quality"><input id="jpeg-quality" v-model.number="draft.export.jpegQuality" type="number" min="1" max="100" step="1" required :disabled="locked" class="w-20 text-right" /></SettingRow>
              <SettingRow :label="t('文件名前缀')" for="filename-prefix" :hint="t('示例：') + exportFilenamePreview(draft.export)"><input id="filename-prefix" v-model="draft.export.filenamePrefix" required :disabled="locked" class="w-44" /></SettingRow>
              <SettingRow :label="t('文件名包含图片尺寸')" for="filename-dimensions"><SwitchControl id="filename-dimensions" v-model="draft.export.includeDimensions" :disabled="locked" @change="captureAutoSave.commit" /></SettingRow>
              <SettingRow :label="t('记住上次保存目录')" for="remember-directory"><SwitchControl id="remember-directory" v-model="draft.export.rememberDirectory" :disabled="locked" @change="captureAutoSave.commit" /></SettingRow>
            </div>
            <p v-if="captureAutoSave.saving.value || captureAutoSave.saved.value" class="settings-save-status" role="status" :class="{ 'status-ok': !captureAutoSave.saving.value }">{{ captureAutoSave.saving.value ? t('正在保存…') : t('已自动保存') }}</p>
            <p v-if="captureAutoSave.error.value" class="notice error" role="alert">{{ te(captureAutoSave.error.value) }} <Button type="button" variant="ghost" @click="captureAutoSave.retry">{{ t('重试') }}</Button></p>
          </form>
        </template>

        <!-- 历史列表在独立窗口；这里只保留唤出入口以及记录、暂存架两组设置。 -->
        <section v-else-if="page === '剪贴板'" class="general-settings-page">
          <ClipboardSettings :disabled="locked" :shortcut="status?.settings.shortcutsEnabled ? status.settings.shortcuts.history : ''" />
          <ShelfSettings />
        </section>
        <DiagnosticsSettings v-else-if="page === '日志'" />
        <template v-else-if="page === '通用'">
          <GeneralSettings>
            <AppearanceSettings :busy="locked" :saved="savedSection === '通用'" @save="saveAppearance" />
          </GeneralSettings>
          <ConfigTransferSettings :prepare-transfer="prepareConfigTransfer" />
        </template>
        <template v-else-if="page === '识别与翻译'">
          <div class="page-tabs"><div class="segmented-control" role="tablist" :aria-label="t('识别与翻译')"><button v-for="tab in recognitionTabs" :key="tab.value" type="button" role="tab" :aria-selected="recognitionTab === tab.value" @click="recognitionTab = tab.value">{{ t(tab.label) }}</button></div></div>
          <OcrSettings v-if="recognitionTab === 'ocr'" />
          <AiProviderSettings v-else-if="recognitionTab === 'translation'" />
          <ServiceSettings v-else @open-tab="recognitionTab = $event" />
        </template>
        <SupportPages v-else-if="page === '关于与帮助'" :version="status?.version" :busy="!status || completingOnboarding" @navigate="navigate" @onboarding="onboardingError = ''; onboarding = true" @project="openProject" />
        <RecordingPanel v-else-if="page === '录屏'" :shortcut="status?.settings.shortcutsEnabled ? status.settings.shortcuts.record : ''" />
        <PinManager v-else-if="page === '贴图'" :shortcut="status?.settings.shortcutsEnabled ? status.settings.shortcuts.pin : ''" />
        <template v-else-if="page === '快捷键'">
          <form @submit.prevent>
            <h2 class="section-heading">{{ t('全局快捷键') }}<button type="button" :disabled="locked" @click="resetShortcuts">{{ t('恢复默认') }}</button></h2>
            <div class="settings-card">
              <div v-for="action in shortcutActions" :key="action.key" class="setting-row" :data-onboarding="action.key === 'capture' ? 'shortcut-settings' : undefined">
                <span>{{ t(action.label) }}</span>
                <div class="flex items-center gap-2">
                  <ShortcutInput v-model="draft.shortcuts[action.key]" :label="action.label" :title="t(action.description)" :disabled="locked" :validate="value => validateShortcuts({ ...draft.shortcuts, [action.key]: value })" @editing="editingShortcut = $event; if (!$event) shortcutAutoSave.flush()" @error="error = $event" @update:model-value="shortcutAutoSave.schedule" />
                  <button v-if="draft.shortcuts[action.key]" type="button" class="shortcut-clear" :disabled="locked" :title="t('禁用')" :aria-label="t('禁用{0}快捷键', { 0: t(action.label) })" @click="draft.shortcuts[action.key] = ''; shortcutAutoSave.commit()"><X :size="10" :stroke-width="3" /></button>
                </div>
              </div>
            </div>
            <p v-if="shortcutAutoSave.saving.value || shortcutAutoSave.saved.value" class="settings-save-status" role="status" :class="{ 'status-ok': !shortcutAutoSave.saving.value }">{{ shortcutAutoSave.saving.value ? t('正在保存…') : t('已自动保存') }}</p>
            <p v-if="shortcutAutoSave.error.value" class="notice error" role="alert">{{ te(shortcutAutoSave.error.value) }} <Button type="button" variant="ghost" @click="shortcutAutoSave.retry">{{ t('重试') }}</Button></p>
          </form>
        </template>

        <ShortcutCompatibility v-show="page === '快捷键'" :disabled="locked" @testing="testingShortcuts = $event">
          <div class="setting-row"><span>{{ t('示例配置') }}</span><Button type="button" variant="outline" :disabled="locked" @click="copyShortcutExample">{{ shortcutExampleCopied ? t('已复制') : t('复制') }}</Button></div>
        </ShortcutCompatibility>
      </div>
    </main>
  </div>
</template>
