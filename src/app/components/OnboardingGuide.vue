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
  <section class="settings-card mb-4" :aria-label='t("新手引导")'>
    <div class="flex items-center justify-between py-3"><h2 class="font-semibold">{{ t('开始使用 Snap Desk') }}</h2><span class="text-xs text-zinc-500">{{ step }} / 3</span></div>
    <div v-if="step === 1" class="space-y-2 text-sm">
      <h3 class="font-medium">{{ t('截图与长截图') }}</h3>
      <ol class="list-decimal space-y-1 pl-5">
        <li>{{ t('点击“开始截图”，拖动框选需要的区域。') }}</li>
        <li>{{ t('在选区工具栏标注、复制或保存；截长图请点“滚动截图”。') }}</li>
        <li>{{ t('滚动截图时慢慢滚动，停一下再继续；点“完成”保存，Esc 取消。') }}</li>
      </ol>
      <Button variant="outline" :disabled="busy" @click="emit('navigate', '截图')">{{ t('去截图') }}</Button>
    </div>
    <div v-else-if="step === 2" class="space-y-2 text-sm"><h3 class="font-medium">{{ t('常用快捷键') }}</h3><div v-for="action in shortcutActions" :key="action.key" class="flex justify-between"><span>{{ t(action.label) }}</span><kbd>{{ settings.shortcuts[action.key] || t('已禁用') }}</kbd></div><p v-if="!settings.shortcutsEnabled">{{ t('全局快捷键总开关当前已关闭。') }}</p><Button variant="outline" :disabled="busy" @click="emit('navigate', '快捷键')">{{ t('修改快捷键') }}</Button></div>
    <div v-else class="space-y-2 text-sm"><h3 class="font-medium">{{ t('保存与退出') }}</h3><p>{{ t('普通截图可选择保存位置；长截图完成后自动保存 PNG 并复制。') }}</p><Button variant="outline" :disabled="busy" @click="emit('navigate', '通用')">{{ t('设置保存目录') }}</Button><p class="text-xs text-zinc-500">{{ t('收起窗口后仍在托盘运行；退出应用请用托盘菜单。') }}</p></div>
    <p class="pt-3 text-xs text-zinc-500">{{ t('以后可从“帮助”重新打开引导。') }}</p>
    <div class="flex items-center gap-3 py-3"><Button variant="ghost" :disabled="busy" @click="emit('complete')">{{ t('跳过引导') }}</Button><div class="flex-1" /><Button v-if="step > 1" variant="outline" :disabled="busy" @click="step--">{{ t('上一步') }}</Button><Button v-if="step < 3" :disabled="busy" @click="step++">{{ t('下一步') }}</Button><Button v-else :disabled="busy" @click="emit('complete')">{{ t('完成') }}</Button></div>
  </section>
</template>
