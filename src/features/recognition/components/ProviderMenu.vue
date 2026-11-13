<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Check, ChevronDown, Cloud, HardDrive, Server } from 'lucide-vue-next'
import { t } from '@/shared/i18n/i18n'
import type { ConnectionMode } from '../services'
import { providerIcon } from '../provider-icons'
/** 引擎下拉：触发按钮显示当前选择，展开后列出全部引擎；只负责选择，配置与保存由所属页面处理。 */
export interface ProviderMenuItem { value: string; label: string; description?: string; disabled?: boolean; connection?: ConnectionMode; active?: boolean; presetId?: string; endpoint?: string }
const props = defineProps<{ modelValue: string; options: ProviderMenuItem[]; label: string; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const open = ref(false), root = ref<HTMLElement | null>(null)
const items = computed(() => props.options.map(item => ({ ...item, ...providerIcon(item.value, item.label, item.presetId, item.endpoint) })))
const current = computed(() => items.value.find(item => item.value === props.modelValue))
const connectionLabel = (connection?: ConnectionMode) => connection === 'offline' ? t('离线') : connection === 'online' ? t('在线 API') : connection ? t('本机 API') : ''
function choose(value: string) { open.value = false; emit('update:modelValue', value) }
function outside(event: PointerEvent) { if (open.value && root.value && !root.value.contains(event.target as Node)) open.value = false }
function key(event: KeyboardEvent) { if (open.value && event.key === 'Escape') { event.stopPropagation(); open.value = false } }
onMounted(() => { if (typeof document !== 'undefined' && document.addEventListener) document.addEventListener('pointerdown', outside) })
onUnmounted(() => { if (typeof document !== 'undefined' && document.removeEventListener) document.removeEventListener('pointerdown', outside) })
</script>

<template>
  <div ref="root" class="provider-select" @keydown="key">
    <button type="button" class="provider-trigger" aria-haspopup="true" :aria-expanded="open" :title="label" :disabled="disabled" @click="open = !open">
      <span v-if="current" class="provider-icon" :class="{ logo: current.icon }" :style="current.icon ? undefined : { background: current.tile }" aria-hidden="true"><span v-if="current.icon" v-html="current.icon" /><template v-else>{{ current.mono }}</template></span>
      <span class="provider-trigger-name">{{ current?.label ?? '' }}</span>
      <span v-if="current?.connection" class="provider-trigger-meta">{{ connectionLabel(current.connection) }}</span>
      <ChevronDown :size="14" aria-hidden="true" />
    </button>
    <nav v-show="open" class="recognition-provider-menu" :aria-label="label">
      <button v-for="item in items" :key="item.value" type="button" :data-provider="item.value" :title="[item.label, item.description, connectionLabel(item.connection), item.active ? t('当前使用') : ''].filter(Boolean).join(' · ')" :aria-current="modelValue === item.value ? 'page' : undefined" :disabled="disabled || item.disabled" @click="choose(item.value)">
        <span class="provider-icon" :class="{ logo: item.icon }" :style="item.icon ? undefined : { background: item.tile }" aria-hidden="true"><span v-if="item.icon" v-html="item.icon" /><template v-else>{{ item.mono }}</template></span>
        <span class="recognition-menu-name">{{ item.label }}<small v-if="item.description">{{ item.description }}</small></span>
        <span class="recognition-menu-meta">
          <span v-if="item.connection" class="recognition-connection-badge" :data-connection="item.connection" :title="connectionLabel(item.connection)"><component :is="item.connection === 'offline' ? HardDrive : item.connection === 'online' ? Cloud : Server" :size="13" aria-hidden="true" /><span class="sr-only">{{ connectionLabel(item.connection) }}</span></span>
          <span class="recognition-active-provider"><span v-if="item.active" data-active-provider :title="t('当前使用')"><Check :size="13" aria-hidden="true" /><span class="sr-only">{{ t('当前使用') }}</span></span></span>
        </span>
      </button>
    </nav>
  </div>
</template>

<style>
/* 触发按钮与下拉列表；识别、翻译页的引擎行和详情区排在同一张分组卡片里。 */
.provider-select { position:relative; }
.provider-trigger { display:inline-flex; align-items:center; gap:8px; min-width:200px; max-width:280px; height:28px; padding:0 9px; border:1px solid var(--desk-line,#d3d8de); border-radius:6px; background:var(--desk-surface,#fff); color:var(--desk-text,#1f2328); font:inherit; font-size:13px; cursor:pointer; }
.provider-trigger:hover:enabled { border-color:var(--desk-muted,#5f6670); }
.provider-trigger:disabled { opacity:.45; cursor:default; }
.provider-trigger > svg { flex:none; color:var(--desk-muted,#5f6670); }
.provider-icon { width:18px; height:18px; flex:none; border-radius:5px; display:inline-flex; align-items:center; justify-content:center; color:#fff; font-size:9px; font-weight:700; letter-spacing:-.02em; line-height:1; }
.provider-icon.logo { background:transparent; color:var(--desk-text,#1f2328); font-size:16px; }
.provider-icon.logo > span { display:flex; }
.provider-icon.logo svg { width:16px; height:16px; }
.provider-trigger-name { flex:1; min-width:0; text-align:left; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.provider-trigger-meta { flex:none; font-size:11px; color:var(--desk-muted,#5f6670); }
.recognition-provider-menu { position:absolute; right:0; top:calc(100% + 4px); z-index:20; display:flex; flex-direction:column; gap:1px; width:max(100%,260px); max-height:300px; overflow-y:auto; padding:4px; border:1px solid var(--desk-border,#e3e6ea); border-radius:8px; background:var(--desk-surface,#fff); box-shadow:0 8px 24px rgba(15,23,42,.14); }
.recognition-provider-menu button { display:flex; gap:8px; align-items:center; width:100%; min-height:30px; padding:4px 8px; border:0; border-radius:6px; background:transparent; color:var(--desk-text,#1f2328); font:inherit; font-size:13px; text-align:left; cursor:pointer; }
.recognition-provider-menu button:hover:enabled { background:var(--desk-hover,rgba(127,127,127,.12)); }
.recognition-provider-menu button[aria-current=page] { background:var(--desk-soft,#eef1fc); color:var(--desk-accent-fg,#3b5bdb); }
.recognition-provider-menu button:focus-visible { outline:2px solid var(--desk-accent,#3b5bdb); outline-offset:-2px; }
.recognition-provider-menu button:disabled { opacity:.5; cursor:default; }
.recognition-menu-name { flex:1; min-width:0; line-height:18px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.recognition-menu-name small { margin-left:6px; font-size:11px; color:var(--desk-muted,#5f6670); }
.recognition-menu-meta { display:flex; align-items:center; gap:6px; flex:none; white-space:nowrap; }
.recognition-active-provider { width:13px; min-height:13px; color:var(--ui-success-fg,#24834b); }
.recognition-connection-badge { display:inline-flex; align-items:center; gap:4px; flex:none; color:var(--desk-muted,#5f6670); font-size:11px; line-height:16px; }
.recognition-connection-badge[data-connection=online] { color:var(--desk-accent-fg,#3b5bdb); }
.recognition-card { overflow:visible; }
.recognition-detail { padding:8px 12px 10px 0; border-top:1px solid var(--desk-sep,#eceef1); }
.recognition-detail:empty, .recognition-detail:not(:has(> *)) { display:none; }
.recognition-detail > .setting-row, .recognition-detail section > .setting-row { padding-right:0; border-bottom:0; }
</style>
