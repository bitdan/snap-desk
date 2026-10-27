<script setup lang="ts">
import { ref } from 'vue'
import { t } from '@/shared/i18n/i18n'
import Button from '@/shared/ui/button/Button.vue'
import { shortcutActions, type Settings } from '@/features/settings/settings'

defineProps<{ settings: Settings; busy: boolean }>()
const emit = defineEmits<{ complete: []; navigate: [page: string] }>()
const step = ref(1)
</script>

<template>
  <!-- 内嵌引导不遮挡操作，用户可以随时直接开始截图。 -->
  <section class="settings-card mb-5 border-blue-200" :aria-label='t("新手引导")'>
    <div class="flex items-center justify-between py-3"><h2 class="font-semibold">{{ t('开始使用 Snap Desk') }}</h2><span class="text-xs text-zinc-500">{{ step }} / 3</span></div>
    <div v-if="step === 1" class="space-y-2 text-sm"><h3 class="font-medium">{{ t('先完成一次截图') }}</h3><p>{{ t('点击“开始截图”，拖动框选范围，然后在工具栏标注、复制或保存。Esc 可取消。') }}</p><p>{{ t('贴图将图片保留在桌面；滚动截图用于滚动内容；录屏先选范围再开始。') }}</p><p class="text-xs text-zinc-500">{{ t('收起窗口后仍在托盘运行，可从托盘退出。') }}</p></div>
    <div v-else-if="step === 2" class="space-y-2 text-sm"><h3 class="font-medium">{{ t('熟悉全局快捷键') }}</h3><div v-for="action in shortcutActions" :key="action.key" class="flex justify-between"><span>{{ t(action.label) }}</span><kbd>{{ settings.shortcuts[action.key] || t('已禁用') }}</kbd></div><p v-if="!settings.shortcutsEnabled">{{ t('全局快捷键总开关当前已关闭。') }}</p><Button variant="outline" @click="emit('navigate', '快捷键')">{{ t('修改快捷键') }}</Button></div>
    <div v-else class="space-y-2 text-sm"><h3 class="font-medium">{{ t('确认保存位置') }}</h3><p>{{ t('截图使用保存对话框，可记住上一次位置；录屏可选择固定目录。配置文件与图片、视频分别存放。') }}</p><Button variant="outline" @click="emit('navigate', '通用')">{{ t('设置保存目录') }}</Button><p class="text-xs text-zinc-500">{{ t('以后可从“帮助”重新打开引导，所有设置均可随时修改。') }}</p></div>
    <div class="flex items-center gap-3 py-4"><Button variant="ghost" :disabled="busy" @click="emit('complete')">{{ t('跳过引导') }}</Button><div class="flex-1" /><Button v-if="step > 1" variant="outline" :disabled="busy" @click="step--">{{ t('上一步') }}</Button><Button v-if="step < 3" :disabled="busy" @click="step++">{{ t('下一步') }}</Button><Button v-else :disabled="busy" @click="emit('complete')">{{ t('完成') }}</Button></div>
  </section>
</template>
