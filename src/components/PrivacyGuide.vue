<script setup lang="ts">
import { t } from '@/lib/i18n'
defineProps<{ platform?: string }>()
</script>

<template>
  <section class="privacy-guide">
    <h2 class="section-heading">{{ t('隐私与权限') }}</h2>
    <p>{{ t('截图、录屏、本地 OCR 和本地翻译在本机处理；下载模型或引擎需要联网。') }}</p>
    <p>{{ t('启用在线翻译后，识别出的文字会发送到你配置的服务商。请勿发送密码或敏感内容，可在“识别与翻译”中切回本地模式。') }}</p>
    <p>{{ t('剪贴板历史默认关闭，开启后文本、图片和文件记录保存到本机。可在通用设置中关闭，并在剪贴板页清理已有记录；关闭监听不会删除历史。') }}</p>
    <p>{{ t('录屏键鼠效果仅在开启后采集输入；日志和导出文件可能包含本地路径，分享前请检查。') }}</p>
    <p v-if="platform === 'windows'">{{ t('Windows：截图通常无需额外授权。麦克风录制需在系统设置 → 隐私和安全性 → 麦克风中允许桌面应用访问。黑屏可能由受保护内容或权限级别不一致引起。') }}</p>
    <p v-else-if="platform === 'macos'">{{ t('macOS：请在系统设置 → 隐私与安全性 → 屏幕与系统音频录制中允许 Snap Desk，变更后重新启动。授权不代表尚未实现的录屏或多屏功能已可用。') }}</p>
    <p v-else-if="platform === 'linux'">{{ t('Linux：截图能力取决于桌面会话；Wayland Portal 尚未接入。授权窗口并不代表当前截图后端可用，可在受支持的 X11 会话中重试。') }}</p>
    <p v-else>{{ t('请从桌面应用查看当前系统的权限指引。') }}</p>
    <p class="text-xs text-zinc-500">{{ t('此处为操作指引，不会自动授予或检测系统权限。拒绝权限后可稍后在系统设置中调整。') }}</p>
  </section>
</template>

<style scoped>
.privacy-guide p { margin: 8px 0; line-height: 1.7; overflow-wrap: anywhere; }
</style>
