<script lang="ts">
// 只缓存公开的资源状态，不保存密钥或未提交的设置草稿。
let latestLocalStatus: import('@/features/recognition/ai').LocalTranslationStatus | null = null
let localStatusRevision = 0
</script>

<script setup lang="ts">
import SelectControl from '@/shared/ui/SelectControl.vue'
import { useAutoSave } from '@/shared/ui/use-auto-save'
import { updateSettings } from '@/features/settings/settings-write'
import { t, te } from '@/shared/i18n/i18n'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '@/services/api'
import { defaultAiSettings, targetLanguages, validateAiSettings } from '@/features/recognition/ai'
import type { LocalTranslationStatus } from '@/features/recognition/ai'
import { Check } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import ResourceSettings from '@/features/resources/components/ResourceSettings.vue'

const draft = ref(defaultAiSettings()), secret = ref(''), configured = ref<boolean | null>(null)
const localStatus = ref<LocalTranslationStatus | null>(latestLocalStatus)
const checking = ref(false)
let disposed = false
const local = computed(() => draft.value.provider === 'local-ctranslate2')
const languages = computed(() => local.value ? targetLanguages.slice(0, 2) : targetLanguages)
watch(local, value => { if (value && !['zh-CN', 'en'].includes(draft.value.targetLanguage)) draft.value.targetLanguage = 'zh-CN' })
const busy = ref(false), ready = ref(false), error = ref(''), message = ref('')
async function loadSettings() {
  try {
    const settings = await api.status()
    if (disposed) return
    draft.value = { ...settings.settings.ai }; ready.value = true
    if (!local.value) {
      const value = await api.aiProviderStatus()
      if (!disposed) configured.value = value
    }
  }
  catch (e) { if (!disposed) error.value = String(e) }
}
async function refreshLocalStatus() {
  const revision = ++localStatusRevision
  checking.value = true
  try {
    const next = await api.translationModelStatus()
    if (revision === localStatusRevision) {
      latestLocalStatus = next
      if (!disposed) localStatus.value = next
    }
  } catch (e) {
    if (revision === localStatusRevision) {
      latestLocalStatus = null
      if (!disposed) { localStatus.value = null; error.value = String(e) }
    }
  } finally { if (!disposed) checking.value = false }
}
onMounted(() => {
  // 配置/凭证与引擎校验独立返回，慢校验或失败不能拖住整个表单。
  void loadSettings()
  void refreshLocalStatus()
})
onUnmounted(() => { disposed = true })
/** 只覆盖 AI 字段，读取最新配置以保留其他页面已保存的设置。密钥从不进入 settings。 */
const autosave = useAutoSave(async () => {
  if (!ready.value) return
  const ai = { ...draft.value }, key = secret.value
  await updateSettings(current => ({ ...current, ai }))
  if (ai.provider !== 'local-ctranslate2' && key) {
    await api.aiCredentialSet(key)
    if (secret.value === key) secret.value = ''
  }
  configured.value = ai.provider === 'local-ctranslate2' ? false : await api.aiProviderStatus()
}, () => validateAiSettings(draft.value))
/** 密钥在结束编辑后保存，避免逐字符写入系统凭据库。 */
function changed(event: Event) {
  if ((event.target as HTMLInputElement).type !== 'password') autosave.schedule()
}
async function credentialAction(action: 'delete' | 'test') {
  await autosave.flush()
  if (autosave.error.value) return
  busy.value = true; error.value = ''; message.value = ''
  try {
    if (action === 'delete') { await api.aiCredentialSet(null); configured.value = false; message.value = '密钥已删除。' }
    else { await api.aiProviderTest(); message.value = '翻译测试成功。' }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 引擎和语言包均由用户主动下载或导入，状态与 OCR 独立。 */
async function modelAction(modelId: string, action: 'download' | 'import' | 'delete') {
  if (busy.value || checking.value) return
  const revision = ++localStatusRevision
  busy.value = true; error.value = ''; message.value = modelId === 'runtime' ? '正在安装引擎…' : ''
  try {
    const next = await api.translationModelAction(modelId, action)
    if (revision === localStatusRevision) latestLocalStatus = next
    if (!disposed) { localStatus.value = next; message.value = modelId === 'runtime' ? '引擎状态已更新。' : '语言包状态已更新。' }
  }
  catch (e) { message.value = ''; error.value = String(e) }
  finally { busy.value = false }
}
</script>

<template>
  <section class="ai-settings">
    <section class="recognition-section">
      <h3>{{ t("文字识别") }}</h3>
      <ResourceSettings kind="ocr" />
    </section>
    <section class="recognition-section translation-section">
    <h3>{{ t("文本翻译") }}</h3>
    <fieldset :disabled="busy || !ready" @input="changed" @change="autosave.commit" @focusout="autosave.flush">
      <label class="provider-field">{{ t("翻译引擎") }}<SelectControl v-model="draft.provider" :disabled="busy || !ready" :aria-label="t('翻译引擎')" :options="[{ value: 'local-ctranslate2', label: t('本地离线') }, { value: 'online-llm', label: t('在线文本服务') }]" @update:model-value="autosave.commit" /></label>
      <div v-if="local" class="local-resources">
        <div class="model-row runtime-row">
          <span class="runtime-status" :class="{ 'status-ok': localStatus?.runtimeReady }" role="status"><Check v-if="localStatus?.runtimeReady" :size="14" />{{ !localStatus ? checking ? t("正在检查…") : t("引擎状态未读取") : localStatus.runtimeReady ? t("本地引擎已安装") : t("翻译引擎未安装或校验失败") }}</span>
          <div class="model-actions">
            <Button variant="outline" :disabled="checking || !localStatus?.runtimeSupported" @click="modelAction('runtime', 'download')">{{ t("下载引擎") }}</Button>
            <Button variant="outline" :disabled="checking || !localStatus?.runtimeSupported" @click="modelAction('runtime', 'import')">{{ t("导入引擎 ZIP") }}</Button>
          </div>
        </div>
        <p v-if="localStatus && !localStatus.runtimeSupported" class="hint">{{ t("翻译引擎下载和导入目前仅支持 Windows x64。") }}</p>
        <div v-for="pack in localStatus?.models" :key="pack.modelId" class="model-row">
          <span class="model-name">{{ pack.modelId.includes('en-zh') ? t("英语 → 简体中文") : t("中文 → 英语") }}</span>
          <span :class="{ 'status-ok': pack.ready }" role="status"><Check v-if="pack.ready" :size="14" />{{ pack.ready ? t("已校验") : t("未安装或校验失败") }}</span>
          <div class="model-actions">
            <Button variant="outline" :disabled="checking || !pack.downloadable" @click="modelAction(pack.modelId, 'download')">{{ t("下载") }}{{ pack.downloadBytes ? `（${Math.ceil(pack.downloadBytes / 1048576)} MiB）` : '' }}</Button>
            <Button variant="outline" :disabled="checking" @click="modelAction(pack.modelId, 'import')">{{ t("导入语言包") }}</Button>
            <Button v-if="pack.revision" variant="ghost" :disabled="checking" @click="modelAction(pack.modelId, 'delete')">{{ t("卸载") }}</Button>
          </div>
        </div>
      </div>
      <div v-else class="online-fields">
        <p class="hint">{{ t("在线翻译会发送原文，可能产生费用。") }}</p>
        <label>{{ t("完整接口地址") }}<input v-model.trim="draft.endpoint" type="url" :placeholder='t("https://服务域名/v1/chat/completions")' autocomplete="off" /></label>
        <label>{{ t("模型名称") }}<input v-model.trim="draft.model" :placeholder='t("填写服务支持的模型 ID")' /></label>
        <label>API Key<input v-model="secret" type="password" autocomplete="new-password" :placeholder='t("留空保留现有密钥")' /></label>
        <p :class="{ 'status-ok': configured }" role="status"><Check v-if="configured" :size="14" />{{ t("密钥") }}{{ configured === null ? t("正在检查…") : configured ? t("已配置") : t("未配置") }}{{ t("（密钥随当前服务地址保存）") }}</p>
      </div>
      <div class="translation-options">
        <label>{{ t("目标语言") }}<SelectControl v-model="draft.targetLanguage" :disabled="busy || !ready" :aria-label="t('目标语言')" :options="languages.map(item => ({ value: item.value, label: t(item.label) }))" @update:model-value="autosave.commit" /></label>
        <label>{{ t("超时（秒）") }}<input v-model.number="draft.timeoutSeconds" type="number" min="5" max="120" /></label>
        <label v-if="local">{{ t("闲置回收（秒）") }}<input v-model.number="draft.localIdleSeconds" type="number" min="0" max="3600" step="1" /><small>{{ t("0 不回收；可设为 30～3600 秒。") }}</small></label>
      </div>
      <div class="actions"><span class="settings-save-status" role="status">{{ autosave.saving.value ? t("正在保存…") : autosave.saved.value ? t("已自动保存") : t("更改自动保存") }}</span><Button variant="outline" :disabled="!local && !configured" @click="credentialAction('test')">{{ t("测试已保存配置") }}</Button><Button v-if="!local" variant="ghost" :disabled="!configured" @click="credentialAction('delete')">{{ t("删除已保存密钥") }}</Button></div>
    </fieldset>
    <p v-if="autosave.error.value" class="notice error" role="alert">{{ te(autosave.error.value) }}<Button variant="ghost" @click="autosave.retry">{{ t("重试") }}</Button></p>
    <p v-if="message" :class="{ 'status-ok': !busy }" role="status"><Check v-if="!busy" :size="14" />{{ t(message) }}</p><p v-if="error" class="notice error" role="alert">{{ te(error) }}</p>
    </section>
  </section>
</template>

<style scoped>
.ai-settings { width:100%; max-width:920px; margin-inline:auto; font-size:13px; }
.recognition-section { padding:16px 0; border-bottom:1px solid #e1e4e8; }
.recognition-section:first-child { padding-top:0; }
.recognition-section:last-child { border-bottom:0; }
h3 { font-size:14px; font-weight:600; margin:0 0 12px; }
p { color:#57606a; margin:8px 0; }
fieldset { border:0; padding:0; min-width:0; }
label { display:grid; grid-template-columns:110px minmax(0,1fr); align-items:center; gap:12px; margin:8px 0; }
input, select { height:32px; border:1px solid #d8dee4; border-radius:4px; padding:4px 8px; min-width:0; width:100%; color:inherit; }
input:focus-visible, select:focus-visible { outline:2px solid #2563eb; outline-offset:2px; }
.provider-field { grid-template-columns:110px minmax(0,240px); margin-top:0; }
.local-resources { margin:12px 0 16px; }
.model-row { display:grid; grid-template-columns:minmax(120px,1fr) minmax(100px,1fr) minmax(0,auto); align-items:center; gap:8px 16px; padding:8px 0; min-height:48px; border-bottom:1px solid #edf0f2; }
.model-row:last-child { border-bottom:0; }
.runtime-status { grid-column:span 2; }
.model-name { overflow-wrap:anywhere; }
.model-actions { display:flex; align-items:center; justify-content:flex-end; flex-wrap:wrap; gap:6px; }
.translation-options { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); align-items:start; gap:16px; max-width:660px; margin:12px 0 16px; }
.translation-options label { display:flex; flex-direction:column; align-items:stretch; gap:6px; margin:0; font-size:12px; color:#57606a; }
.translation-options :is(input,select) { font-size:13px; color:#24292f; }
.translation-options small { line-height:1.5; font-size:12px; }
.online-fields { max-width:660px; }
.actions { display:flex; align-items:center; flex-wrap:wrap; gap:8px; padding-top:4px; }
.hint { font-size:12px; line-height:1.6; }
.error { color:#b91c1c; }
.ai-settings :deep(.setting-row) { min-height:48px; padding-top:0; }
[data-theme=dark] .recognition-section, [data-theme=dark] .model-row { border-color:#39424e; }
[data-theme=dark] :is(p,.translation-options label) { color:#a6b2c0; }
[data-theme=dark] .translation-options :is(input,select) { color:#e6edf3; }
[data-theme=dark] .error { color:#ffb8bd; }
@media (max-width:760px) {
  .model-row { grid-template-columns:minmax(0,1fr) auto; }
  .runtime-status { grid-column:span 2; }
  .model-actions { grid-column:span 2; justify-content:flex-start; }
}
@media (max-width:540px) {
  .translation-options { grid-template-columns:1fr; gap:12px; }
  .provider-field, .online-fields label { grid-template-columns:1fr; gap:6px; }
}
</style>
