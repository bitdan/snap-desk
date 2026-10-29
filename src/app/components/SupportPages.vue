<script setup lang="ts">
import { Camera, Video, Pin, Folder, ArrowUpRight, Scan, Github } from 'lucide-vue-next'
import Button from '@/shared/ui/button/Button.vue'
import { t } from '@/shared/i18n/i18n'
import type { Shortcuts } from '@/features/settings/settings'
defineProps<{ page: string; version?: string; shortcuts?: Shortcuts; busy: boolean }>()
const emit = defineEmits<{ navigate: [page: string]; onboarding: []; project: [] }>()
// 帮助直接连接到可操作页面，快捷键使用已生效配置。
const guides = [
  { page: '截图', icon: Camera, key: 'capture', title: '框选与标注', text: '框选后使用工具栏标注、复制或保存；滚动截图也从选区工具栏开始。' },
  { page: '录屏', icon: Video, key: 'record', title: '录制屏幕', text: '设置格式与画质，选择范围后确认声音。录制中可暂停，停止后自动导出。' },
  { page: '贴图', icon: Pin, key: 'pin', title: '桌面参考图', text: '把剪贴板文字、图片或本地图片贴在桌面，在贴图页管理显示和分组。' },
  { page: '剪贴板', icon: Folder, key: 'history', title: '查找复制记录', text: '按类型筛选或搜索，右键记录进行复制、收藏或删除。记录范围在通用设置中调整。' },
] as const
</script>

<template>
  <section v-if="page === '帮助'" class="help-page">
    <div class="tool-actionbar"><span class="tool-context">{{ t('常用操作') }}</span><Button variant="outline" :disabled="busy" @click="emit('onboarding')">{{ t('重新打开新手引导') }}</Button></div>
    <button v-for="guide in guides" :key="guide.page" class="help-row" @click="emit('navigate', guide.page)">
      <span class="help-icon"><component :is="guide.icon" :size="20" /></span>
      <span class="help-copy"><strong>{{ t(guide.title) }}</strong><span>{{ t(guide.text) }}</span></span>
      <kbd v-if="shortcuts?.[guide.key]">{{ shortcuts[guide.key] }}</kbd><ArrowUpRight :size="16" />
    </button>
    <h2 class="section-heading">{{ t('使用与排错') }}</h2>
    <div class="setting-row"><span>{{ t('快捷键没有响应') }}<small>{{ t('检查是否启用、按键冲突与兼容性。') }}</small></span><Button variant="outline" @click="emit('navigate', '快捷键')">{{ t('检查快捷键') }}</Button></div>
    <div class="setting-row"><span>{{ t('识别或翻译不可用') }}<small>{{ t('检查资源与服务配置。') }}</small></span><Button variant="outline" @click="emit('navigate', '识别与翻译')">{{ t('识别与翻译') }}</Button></div>
    <div class="setting-row"><span>{{ t('遇到其他问题') }}<small>{{ t('查看日志，或导出日志以协助排查。') }}</small></span><Button variant="outline" @click="emit('navigate', '日志')">{{ t('查看日志') }}</Button></div>
    <p class="support-note">{{ t('Esc 收起主窗口，应用继续在托盘运行。退出会清空贴图，请先保存需要的内容。') }}</p>
  </section>
  <section v-else class="about-page">
    <div class="about-identity"><span class="brand-icon"><Scan :size="28" /></span><div><h2>Snap Desk</h2><span class="pill">{{ version ? `v${version}` : t('桌面版本待读取') }}</span></div></div>
    <p class="support-note">{{ t('截图、标注、贴图、滚动截图与录屏，本地处理。') }}</p>
    <h2 class="section-heading">{{ t('应用信息') }}</h2>
    <div class="setting-row"><span>{{ t('开源许可') }}</span><span>GPL-3.0-only</span></div>
    <div class="setting-row"><span>{{ t('录屏系统要求') }}</span><span>Windows 10 2004+</span></div>
    <div class="setting-row"><span>{{ t('项目主页') }}<small>github.com/bitdan/snap-desk</small></span><Button variant="outline" @click="emit('project')"><Github :size="16" />GitHub<ArrowUpRight :size="14" /></Button></div>
    <div class="setting-row"><span>{{ t('需要帮助') }}</span><Button variant="ghost" @click="emit('navigate', '帮助')">{{ t('查看使用帮助') }}</Button></div>
  </section>
</template>

<style scoped>
.help-row { display:flex; width:100%; align-items:center; gap:16px; padding:18px 8px; text-align:left; border-bottom:1px solid var(--desk-border); border-radius:6px; }
.help-row:hover { background:var(--desk-soft); }
.help-icon { display:flex; color:var(--desk-accent); padding:10px; background:var(--desk-soft); border-radius:8px; flex:none; }
.help-copy { flex:1; min-width:0; display:grid; gap:6px; }
.help-copy strong { font-weight:600; }
.help-copy > span, .support-note { color:var(--desk-muted); font-size:12px; line-height:1.8; }
.help-row > svg { flex:none; color:var(--desk-muted); }
.support-note { margin:18px 0; }
.about-identity { display:flex; align-items:center; gap:16px; padding:12px 0; }
.about-identity .brand-icon { width:56px; height:56px; border-radius:12px; }
.about-identity h2 { font-size:22px; font-weight:600; margin:0 0 6px; letter-spacing:-.5px; }
@media (max-width:640px) { .help-row { gap:10px; flex-wrap:wrap; } .help-copy { flex-basis:60%; } }
</style>
