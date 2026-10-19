<script setup lang="ts">
import ScrollCaptureIcon from './ScrollCaptureIcon.vue'
import { onMounted, ref } from 'vue'
import { X } from 'lucide-vue-next'

defineProps<{ busy: boolean; error: string }>()
const emit = defineEmits<{ confirm: []; close: [] }>()
const root = ref<HTMLElement | null>(null)
// 二级面板打开后约束键盘焦点，避免 Tab 回到背后的截图工具栏。
function trapFocus(event: KeyboardEvent) {
  if (event.key !== 'Tab') return
  const items = [...(root.value?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)') ?? [])]
    .filter(el => el.getClientRects().length && el.tabIndex !== -1)
  const first = items[0], last = items.at(-1)
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
}
onMounted(() => root.value?.querySelector<HTMLButtonElement>('.start')?.focus())
</script>

<template>
  <section ref="root" class="scroll-menu" role="dialog" aria-modal="true" aria-label="长截图选项" @keydown="trapFocus" @pointerdown.stop @pointermove.stop @contextmenu.stop.prevent>
    <header><strong>长截图</strong><button :disabled="busy" title="关闭长截图选项" aria-label="关闭长截图选项" @click="emit('close')"><X :size="16" /></button></header>
    <div class="options">
      <p class="hint">手动向下滚动，停顿后拼接</p>
    </div>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <footer><button class="start" :disabled="busy" title="开始采集当前选区" @click="emit('confirm')"><ScrollCaptureIcon :size="15" />{{ busy ? '准备中…' : '开始长截图' }}</button></footer>
  </section>
</template>

<style scoped>
.scroll-menu { position: fixed; width: min(300px, calc(100vw - 16px)); overflow-y: auto; border: 1px solid #e4e4e7; border-radius: 9px; background: white; color: #27272a; font-size: 12px; box-shadow: 0 8px 32px #0003; }
header, footer { display: flex; align-items: center; justify-content: space-between; padding: 10px 12px; }
header { border-bottom: 1px solid #eee; } header button { display: flex; padding: 4px; border-radius: 4px; }
.options { padding: 12px; display: grid; gap: 12px; }.hint { color: #71717a; margin: 0; }
label { color: #71717a; }
footer { border-top: 1px solid #eee; justify-content: flex-end; }.start { display: flex; align-items: center; gap: 6px; background: #7c3aed; color: white; border-radius: 5px; padding: 7px 12px; }
.error { padding: 8px 12px; color: #be123c; background: #fff1f2; overflow-wrap: anywhere; }
button:disabled { opacity: .5; } :focus-visible { outline: 2px solid #a78bfa; outline-offset: 2px; }
</style>
