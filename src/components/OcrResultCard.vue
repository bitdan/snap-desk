<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import { Check, Copy, Languages, ScanText, X } from 'lucide-vue-next'
import Button from './ui/button/Button.vue'
import { api } from '@/lib/api'
import './recognition-results.css'

// 草稿由截图会话持有，切换到翻译或返回识别时保留用户修正。
const text = defineModel<string>('text', { required: true })
const emit = defineEmits<{ close: []; translate: [] }>()
const busy = ref(false), message = ref(''), error = ref('')
const count = computed(() => Array.from(text.value).length)
const input = ref<HTMLTextAreaElement | null>(null)
onMounted(() => { void nextTick(() => input.value?.focus()) })
async function copy() {
  if (busy.value || !text.value.trim()) return
  busy.value = true; message.value = ''; error.value = ''
  try { await api.copyOcrText(text.value); message.value = '文字已复制。' }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function key(event: KeyboardEvent) {
  if (event.key === 'Escape' && !busy.value) { event.preventDefault(); emit('close') }
  // 焦点限制在结果面板中，避免 Tab 移到截图工具栏后意外保存或关闭会话。
  if (event.key === 'Tab') {
    const nodes = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('textarea:not(:disabled), button:not(:disabled)')]
    const first = nodes[0], last = nodes[nodes.length - 1]
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
  }
}
</script>

<template>
  <section class="recognition-result" @keydown.stop="key">
    <header class="result-header"><h2><ScanText :size="17" />文字识别</h2><Button class="result-close" variant="ghost" :disabled="busy" title="关闭（Esc）" aria-label="关闭文字识别" @click="emit('close')"><X :size="16" /></Button></header>
    <div class="result-body">
      <div class="result-editor">
        <div class="editor-heading"><label for="ocr-result-text">识别结果</label><span>可编辑 · {{ count }} 字符</span></div>
        <textarea id="ocr-result-text" ref="input" v-model="text" aria-label="识别文字，可编辑" :disabled="busy" placeholder="未识别到文字，可输入文字或重新框选。" spellcheck="false" @input="message = ''; error = ''" />
      </div>
      <p v-if="error" class="result-error" role="alert">{{ error }}</p>
    </div>
    <footer class="result-footer"><span v-if="message" class="result-success" role="status"><Check :size="14" />{{ message }}</span><div class="result-actions"><Button variant="outline" :disabled="busy || !text.trim()" @click="emit('translate')"><Languages :size="15" />翻译文字</Button><Button :disabled="busy || !text.trim()" @click="copy"><Copy :size="15" />复制文字</Button></div></footer>
  </section>
</template>
