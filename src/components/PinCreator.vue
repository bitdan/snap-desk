<script setup lang="ts">
import { ref } from 'vue'
import Button from '@/components/ui/button/Button.vue'
import { api } from '@/lib/api'
import { colorPinRequest, textPinRequest } from '@/lib/pin-content'

defineProps<{ disabled: boolean }>()
const emit = defineEmits<{ create: [task: () => Promise<unknown>] }>()
const kind = ref<'text' | 'color'>('text')
const text = ref('')
const foreground = ref('#27272A')
const background = ref('#FFF4B8')
const fontSize = ref(24)
const color = ref('#7C3AED')
const width = ref(320), height = ref(200)

function create() {
  // 先把整个排版/调用事务交给父层，确保字体加载期间也禁用重复提交与其他管理操作。
  emit('create', async () => {
    if (kind.value === 'text') await api.pinText(await textPinRequest(text.value, foreground.value, background.value, fontSize.value))
    else await api.pinColor(colorPinRequest(color.value, width.value, height.value))
  })
}
</script>

<template>
  <details class="pin-creator settings-card">
    <summary>新建文本 / 颜色贴图</summary>
    <form @submit.prevent="create">
      <fieldset :disabled="disabled">
        <div class="creator-row">
          <label><input v-model="kind" type="radio" value="text" name="pin-kind" /> 文本</label>
          <label><input v-model="kind" type="radio" value="color" name="pin-kind" /> 颜色</label>
        </div>
        <template v-if="kind === 'text'">
          <label class="text-label">文本内容（最多 500 字符）<textarea v-model="text" rows="4" placeholder="输入便签、待办或参考文字" required /></label>
          <div class="creator-row">
            <label>文字色 <input v-model="foreground" aria-label="文字色" type="color" /></label>
            <label>背景色 <input v-model="background" aria-label="文本背景色" type="color" /></label>
            <label>字号 <input v-model.number="fontSize" aria-label="文本字号" type="number" min="8" max="128" step="1" required /></label>
          </div>
          <p>自动换行；贴出后可用“选择 / 编辑”修改文字、颜色和字号。</p>
        </template>
        <template v-else>
          <div class="creator-row">
            <label>颜色 <input v-model="color" aria-label="颜色贴图色值" type="text" maxlength="7" required /></label>
            <input v-model="color" aria-label="选择贴图颜色" type="color" />
            <label>宽 <input v-model.number="width" aria-label="颜色贴图宽度" type="number" min="16" max="2048" step="1" required /></label>
            <label>高 <input v-model.number="height" aria-label="颜色贴图高度" type="number" min="16" max="2048" step="1" required /></label>
          </div>
          <p>色值格式 #RRGGBB；宽高单位为原图像素，范围 16～2048。</p>
        </template>
        <Button type="submit" :disabled="disabled">创建{{ kind === 'text' ? '文本' : '颜色' }}贴图</Button>
      </fieldset>
    </form>
  </details>
</template>

<style scoped>
.pin-creator { padding: 12px 16px; margin-bottom: 16px; }
summary { cursor: pointer; font-size: 13px; font-weight: 600; }
fieldset { border: 0; padding: 12px 0 0; min-width: 0; }
.creator-row { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; margin-bottom: 12px; }
label { font-size: 12px; display: inline-flex; align-items: center; gap: 6px; }
.text-label { display: flex; flex-direction: column; align-items: stretch; margin-bottom: 12px; }
input[type=number], input[type=text], textarea { border: 1px solid #d4d4d8; border-radius: 5px; padding: 6px; background: white; }
input[type=number] { width: 76px; } input[type=text] { width: 92px; }
input[type=color] { width: 32px; height: 28px; }
textarea { width: 100%; resize: vertical; }
p { font-size: 12px; color: #6e7781; margin-bottom: 12px; }
</style>
