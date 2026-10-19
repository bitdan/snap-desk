<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { api } from '@/lib/api'
import { defaultAiSettings, targetLanguages, validateAiSettings } from '@/lib/ai'
import type { LocalTranslationStatus } from '@/lib/ai'
import Button from './ui/button/Button.vue'
import ResourceSettings from './ResourceSettings.vue'

const draft = ref(defaultAiSettings()), secret = ref(''), configured = ref(false)
const localStatus = ref<LocalTranslationStatus | null>(null)
const local = computed(() => draft.value.provider === 'local-ctranslate2')
const languages = computed(() => local.value ? targetLanguages.slice(0, 2) : targetLanguages)
watch(local, value => { if (value && !['zh-CN', 'en'].includes(draft.value.targetLanguage)) draft.value.targetLanguage = 'zh-CN' })
const busy = ref(false), ready = ref(false), error = ref(''), message = ref('')
onMounted(async () => {
  try {
    draft.value = { ...(await api.status()).settings.ai }; ready.value = true
    localStatus.value = await api.translationModelStatus()
    configured.value = local.value ? false : await api.aiProviderStatus()
  }
  catch (e) { error.value = String(e) }
})
/** 只覆盖 AI 字段，读取最新配置以保留其他页面已保存的设置。密钥从不进入 settings。 */
async function save() {
  if (busy.value || !ready.value) return
  busy.value = true; error.value = ''; message.value = ''
  try {
    const invalid = validateAiSettings(draft.value)
    if (invalid) throw new Error(invalid)
    const current = (await api.status()).settings
    await api.saveSettings({ ...current, ai: { ...draft.value } })
    if (!local.value && secret.value) { await api.aiCredentialSet(secret.value); secret.value = '' }
    configured.value = local.value ? false : await api.aiProviderStatus()
    message.value = '翻译设置已保存。'
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function credentialAction(action: 'delete' | 'test') {
  busy.value = true; error.value = ''; message.value = ''
  try {
    if (action === 'delete') { await api.aiCredentialSet(null); configured.value = false; message.value = '已删除已保存服务的密钥。' }
    else { await api.aiProviderTest(); message.value = '已保存配置的文本翻译测试成功。' }
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
/** 用户主动下载或导入，语言包状态与 OCR 独立。 */
async function modelAction(modelId: string, action: 'download' | 'import' | 'delete') {
  busy.value = true; error.value = ''; message.value = ''
  try { localStatus.value = await api.translationModelAction(modelId, action); message.value = '语言包状态已更新。' }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
</script>

<template>
  <section class="ai-settings">
    <h3>文字识别</h3>
    <ResourceSettings kind="ocr" />
    <h3>文本翻译</h3>
    <fieldset :disabled="busy || !ready">
      <label>翻译引擎<select v-model="draft.provider"><option value="local-ctranslate2">本地离线 · CTranslate2 / OPUS-MT</option><option value="online-llm">在线文本服务</option></select></label>
      <template v-if="local">
        <p>英译中 / 中译英；安装语言包后无需网络或 API Key。</p>
        <p>{{ localStatus?.runtimeReady ? '本地引擎文件已安装，可测试实际加载。' : '当前安装缺少原生翻译引擎，请安装带引擎的版本。' }}</p>
        <div v-for="pack in localStatus?.models" :key="pack.modelId" class="model-row">
          <span>{{ pack.modelId.includes('en-zh') ? '英语 → 简体中文' : '中文 → 英语' }} · {{ pack.ready ? '已校验' : '未安装或校验失败' }}</span>
          <Button variant="outline" :disabled="!pack.downloadable" @click="modelAction(pack.modelId, 'download')">下载{{ pack.downloadBytes ? `（${Math.ceil(pack.downloadBytes / 1048576)} MiB）` : '' }}</Button>
          <Button variant="outline" @click="modelAction(pack.modelId, 'import')">导入语言包</Button>
          <Button v-if="pack.revision" variant="ghost" @click="modelAction(pack.modelId, 'delete')">卸载</Button>
        </div>
        <p class="hint">仅导入可信来源的语言包目录。未发布的语言包禁用下载；导入校验大小和 SHA-256，不能证明发布者身份。</p>
      </template>
      <template v-else>
      <p>每次翻译前确认发送原文，可能产生服务费用。</p>
      <label>完整接口地址<input v-model.trim="draft.endpoint" type="url" placeholder="https://服务域名/v1/chat/completions" autocomplete="off" /></label>
      <label>模型名称<input v-model.trim="draft.model" placeholder="填写服务支持的模型 ID" /></label>
      </template>
      <label>目标语言<select v-model="draft.targetLanguage"><option v-for="item in languages" :key="item.value" :value="item.value">{{ item.label }}</option></select></label>
      <label>超时（秒）<input v-model.number="draft.timeoutSeconds" type="number" min="5" max="120" /></label>
      <label v-if="!local">API Key<input v-model="secret" type="password" autocomplete="new-password" placeholder="留空保留密钥；输入后点击保存" /></label>
      <p v-if="!local">已保存服务的密钥：{{ configured ? '已配置' : '未配置' }}。仅保存在 Windows 系统凭证库，更换地址需重新配置。</p>
      <div class="actions"><Button @click="save">保存</Button><Button variant="outline" :disabled="!local && !configured" @click="credentialAction('test')">测试已保存配置</Button><Button v-if="!local" variant="ghost" :disabled="!configured" @click="credentialAction('delete')">删除已保存密钥</Button></div>
    </fieldset>
    <p v-if="!local" class="hint">连接测试会发送固定文本 Hello，可能产生少量费用。翻译失败不会自动切换服务。</p>
    <p v-if="message" role="status">{{ message }}</p><p v-if="error" class="error" role="alert">{{ error }}</p>
  </section>
</template>

<style scoped>
.ai-settings { max-width: 760px; font-size: 13px; }
h3 { font-size: 14px; font-weight: 600; margin: 0 0 8px; }
h3:not(:first-child) { border-top: 1px solid #e1e4e8; margin-top: 16px; padding-top: 16px; }
p { color: #57606a; margin: 8px 0; }
fieldset { border: 0; padding: 0; min-width: 0; }
label { display: grid; grid-template-columns: 120px minmax(0, 1fr); align-items: center; gap: 12px; margin: 12px 0; }
input, select { height: 32px; border: 1px solid #d8dee4; border-radius: 4px; padding: 4px 8px; min-width: 0; width: 100%; }
input:focus-visible, select:focus-visible { outline: 2px solid #2563eb; }
.model-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin: 8px 0; }
.actions { display: flex; flex-wrap: wrap; gap: 8px; }
.hint { font-size: 12px; }.error { color: #b91c1c; }
</style>
