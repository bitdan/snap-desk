<script setup lang="ts">
import { ArrowUpRight, Scan, Github } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import ToolHero from '@/shared/ui/ToolHero.vue'
import { t } from '@/shared/i18n/i18n'
import UpdateSettings from '@/features/settings/components/UpdateSettings.vue'
defineProps<{ version?: string; busy: boolean }>()
const emit = defineEmits<{ navigate: [page: string]; onboarding: []; project: [] }>()
</script>

<template>
  <section class="support-page">
    <ToolHero :icon="Scan" title="Snap Desk" :meta="version ? `v${version}` : t('桌面版本待读取')" />
    <UpdateSettings />
    <h2 class="section-heading">{{ t('帮助') }}</h2>
    <div class="settings-card">
      <div class="setting-row"><span>{{ t('新手引导') }}</span><Button variant="outline" :disabled="busy" @click="emit('onboarding')">{{ t('重新打开新手引导') }}</Button></div>
      <div class="setting-row"><span>{{ t('快捷键没有响应') }}</span><Button variant="outline" @click="emit('navigate', '快捷键')">{{ t('检查快捷键') }}</Button></div>
      <div class="setting-row"><span>{{ t('识别或翻译不可用') }}</span><Button variant="outline" @click="emit('navigate', '文字识别')">{{ t('文字识别') }}</Button></div>
      <div class="setting-row"><span>{{ t('遇到其他问题') }}</span><Button variant="outline" @click="emit('navigate', '日志')">{{ t('查看日志') }}</Button></div>
    </div>
    <p class="section-note">{{ t('Esc 收起主窗口，应用继续在托盘运行。退出会清空贴图，请先保存需要的内容。') }}</p>
    <h2 class="section-heading">{{ t('应用信息') }}</h2>
    <div class="settings-card">
      <div class="setting-row"><span>{{ t('开源许可') }}</span><span class="text-zinc-500">GPL-3.0-only</span></div>
      <div class="setting-row"><span>{{ t('录屏系统要求') }}</span><span class="text-zinc-500">Windows 10 2004+</span></div>
      <div class="setting-row"><span>{{ t('项目主页') }}<small>github.com/bitdan/snap-desk</small></span><Button variant="outline" @click="emit('project')"><Github :size="15" />GitHub<ArrowUpRight :size="14" /></Button></div>
    </div>
  </section>
</template>
