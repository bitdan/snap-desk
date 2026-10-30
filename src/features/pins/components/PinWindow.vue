<script setup lang="ts">
import SelectControl from '@/shared/ui/SelectControl.vue'
import { handleWindowEscape as handleEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
import { t, te } from '@/shared/i18n/i18n'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { Pin, PinOff, Lock, Unlock, Minus, Plus, Copy, Save, EyeOff, X, ChevronDown, ChevronUp, Move, Check } from 'lucide-vue-next'
import PinRecognition from '@/features/recognition/components/PinRecognition.vue'
import AnnotationSurface from '@/features/annotations/components/AnnotationSurface.vue'
import AnnotationToolbar from '@/features/annotations/components/AnnotationToolbar.vue'
import { useFloatingToolbar } from '@/features/annotations/toolbar-layout'
import { useAnnotations, type AnnotationTool } from '@/features/annotations/use-annotations'
import { api, onPinVisibility } from '@/services/api'
import { usePinImage } from '@/features/pins/pin-image'
import type { PinView, PinAction, PinTransform } from '@/features/pins/types'
import { canEditPin, minimumZoom, pinDisplaySize, pinImagePadding, stepZoom, defaultPinAppearance, pinAppearanceStyle } from '@/features/pins/pins'
import { onPinAppearance, onPinView } from '@/features/pins/events'
import layout from '@/features/pins/pin-layout.json'

const toolbar = ref<HTMLElement | null>(null)
const { style: floatingToolbarStyle, start: floatToolbar, reset: resetToolbarPosition } = useFloatingToolbar(toolbar)
const imageSession = usePinImage(knownRevision => api.currentPin(knownRevision))
const { frame } = imageSession
// 首次 IPC 和图片解码完成前不绘制 20px 占位边框，避免贴图看起来先缩小再展开。
const imageReady = ref(false)
watch(() => frame.value?.dataUrl, () => { imageReady.value = false }, { flush: 'sync' })
function imageLoaded(event: Event) {
  imageReady.value = !!frame.value?.dataUrl && (event.currentTarget as HTMLImageElement).src === frame.value.dataUrl
}
const appearance = ref(defaultPinAppearance())
let appearanceRevision = 0
let offAppearance: (() => void) | undefined
let offVisibility: (() => void) | undefined
let offView: (() => void) | undefined
let disposed = false
const busy = ref(false)
const recognitionMode = ref<'ocr' | 'qr' | 'translate' | null>(null)
let recognitionTrigger: HTMLElement | null = null
async function recognize(mode: 'ocr' | 'qr' | 'translate', event: MouseEvent) {
  if (!frame.value || busy.value || editor.pending || editor.interacting) return
  recognitionTrigger = event.currentTarget as HTMLElement
  closeMenu()
  const revision = frame.value.revision
  await run(async () => {
    await api.setPinRecognitionOpen(true)
    if (disposed) return
    if (!frame.value?.dataUrl || frame.value.revision !== revision) {
      await api.setPinRecognitionOpen(false)
      return
    }
    recognitionMode.value = mode
  })
}
async function closeRecognition(restoreFocus = true) {
  if (!recognitionMode.value) return
  recognitionMode.value = null
  try { await api.setPinRecognitionOpen(false) }
  catch (e) { if (!disposed) error.value = String(e) }
  if (restoreFocus && !disposed) void nextTick(() => recognitionTrigger?.isConnected ? recognitionTrigger.focus() : menuTrigger?.focus())
}
const error = ref('')
const message = ref('')
const controls = computed(() => frame.value?.view.toolbarExpanded ?? true)
const controlsHovered = ref(false)
let controlsHideTimer: ReturnType<typeof setTimeout> | undefined
function keepControlsVisible() {
  clearTimeout(controlsHideTimer)
  controlsHovered.value = true
}
function hideControls() {
  clearTimeout(controlsHideTimer)
  if (!document.querySelector('.select-menu[data-state="open"]')) controlsHovered.value = false
}
// 留出穿过图片与底栏间隙的时间；隐藏的底栏不接收指针事件，不能自行唤出。
function scheduleControlsHide() {
  clearTimeout(controlsHideTimer)
  controlsHideTimer = setTimeout(hideControls, 180)
}
const viewport = ref({ width: window.innerWidth, height: window.innerHeight, pixelRatio: window.devicePixelRatio })
const display = computed(() => pinDisplaySize(frame.value?.width ?? 20, frame.value?.height ?? 20,
  frame.value?.view.zoom ?? 1, viewport.value.width, viewport.value.height, controls.value, viewport.value.pixelRatio))
const focused = ref(document.hasFocus())
const annotating = ref(false)
const contextMenu = ref<{ x: number; y: number } | null>(null)
useEscapeLayer(() => contextMenu.value !== null, () => closeMenu(true))
const menuElement = ref<HTMLElement | null>(null)
let menuTrigger: HTMLElement | null = null
const bounds = computed(() => ({ x: 0, y: 0, width: frame.value?.width ?? 0, height: frame.value?.height ?? 0 }))
const editor = useAnnotations(() => bounds.value, () => frame.value?.dataUrl ?? '', () => busy.value || !!frame.value?.view.locked, async shapes => {
  await api.setPinAnnotations(shapes)
  if (frame.value) frame.value.annotations = shapes
})
// 图片和底栏是两个网格区域；阴影占用外边距，底栏不再浮在图片上。
const shellStyle = computed(() => ({
  padding: `${pinImagePadding(viewport.value.pixelRatio)}px`, gap: `${layout.gap}px`,
  // 显式限定列宽；长工具栏不能撑大隐式网格列，把居中的图片推到窗口外。
  gridTemplateColumns: 'minmax(0, 1fr)',
  gridTemplateRows: `minmax(${layout.minVisibleSide + layout.border * 2}px, 1fr) ${controls.value ? layout.expandedHeight : layout.collapsedHeight}px`,
}))
const stageStyle = computed(() => ({
  cursor: editable.value ? 'move' : 'default', borderWidth: `${layout.border}px`,
  // 使用确定的像素尺寸，边框与内容一起缩放；大图在边框内滚动查看。
  width: `${display.value.stageWidth}px`, height: `${display.value.stageHeight}px`,
}))
const editable = computed(() => canEditPin(frame.value?.view, busy.value || editor.pending || editor.interacting))
const transforms: { action: PinTransform; label: string }[] = [
  { action: 'rotateLeft', label: '左转' }, { action: 'rotateRight', label: '右转' },
  { action: 'flipHorizontal', label: '水平翻转' }, { action: 'flipVertical', label: '垂直翻转' },
  { action: 'grayscale', label: '灰度' }, { action: 'invert', label: '反色' }, { action: 'reset', label: '恢复原图' },
]
/** 所有系统操作经过 API 层；失败保留当前图片并允许重试。 */
async function run(task: () => Promise<void>) {
  if (busy.value || editor.pending || editor.interacting) return
  busy.value = true; error.value = ''; message.value = ''
  try { await task() } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function load() {
  await run(async () => {
    const revision = appearanceRevision
    if (await imageSession.refresh()) editor.reset(frame.value!.annotations)
    if (frame.value && revision === appearanceRevision) appearance.value = frame.value.appearance
  })
}
async function pinVisibility(visible: boolean) {
  imageSession.visibility(visible)
  if (!visible) { void closeRecognition(false); closeMenu(); hideControls(); return }
  const revision = appearanceRevision
  try {
    if (await imageSession.refresh()) editor.reset(frame.value!.annotations)
    if (frame.value && revision === appearanceRevision) appearance.value = frame.value.appearance
  }
  catch (e) { if (!disposed) error.value = String(e) }
}
async function setView(patch: Partial<PinView>) {
  if (!frame.value) return
  await run(async () => { frame.value!.view = await api.setPinView({ ...frame.value!.view, ...patch }) })
}
function toggleControls() { void setView({ toolbarExpanded: !controls.value }) }
function updateFocus() { focused.value = document.hasFocus(); if (!focused.value) closeMenu() }
function closeMenu(restoreFocus = false) {
  contextMenu.value = null
  if (restoreFocus) menuTrigger?.focus({ preventScroll: true })
}
/** 选中标注工具时先为底栏留出空间；右键菜单仅保留图片和窗口操作。 */
async function startEditing(tool: AnnotationTool = 'edit') {
  closeMenu(true)
  if (!editable.value) return
  if (!controls.value) await setView({ toolbarExpanded: true })
  if (!controls.value || !editable.value) return
  annotating.value = true
  editor.setTool(tool)
}
async function openMenu(event: MouseEvent) {
  event.preventDefault(); event.stopPropagation()
  if (!frame.value || editor.pending || busy.value) return
  // 右键仍可放弃未完成的笔画；完成后再次右键才显示菜单。
  if (editor.interacting) { editor.abort(); return }
  menuTrigger = event.currentTarget as HTMLElement
  contextMenu.value = { x: event.clientX, y: event.clientY }
  await nextTick()
  const box = menuElement.value?.getBoundingClientRect()
  if (contextMenu.value && box) {
    contextMenu.value = { x: Math.max(4, Math.min(event.clientX, window.innerWidth - box.width - 4)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - box.height - 4)) }
    menuElement.value?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  }
}
function dismissMenu(event: PointerEvent) {
  if (!menuElement.value?.contains(event.target as Node)) closeMenu()
}
function menuKey(event: KeyboardEvent) {
  if (handleEscape(event, () => closeMenu(true))) return
  if (event.key === 'Tab') { event.preventDefault(); closeMenu(true); return }
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  const buttons = Array.from(menuElement.value?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length
  buttons[next]?.focus()
}
async function action(value: PinAction) {
  await run(async () => {
    const result = await api.pinAction(value)
    message.value = value === 'copy' ? '已复制' : value === 'save' ? result ? '图片已保存' : '已取消保存' : ''
  })
}
async function transform(value: PinTransform) {
  if (!editable.value) return
  await run(async () => {
    if (!imageSession.replace(await api.transformPin(value)) || !frame.value) return
    editor.reset(frame.value.annotations)
    // 旋转可能交换宽高，按现有比例重新适配窗口；像素结果已由 Rust 合成。
    frame.value.view = await api.setPinView(frame.value.view)
  })
}
const minZoom = computed(() => frame.value ? minimumZoom(frame.value.width, frame.value.height, viewport.value.pixelRatio) : 1)
function zoom(direction: number) {
  if (!editable.value || !frame.value) return
  const next = stepZoom(frame.value.view.zoom, direction, minZoom.value)
  // 到达边界后不再调用原生窗口缩放，连续滚动也保持当前窗口可见。
  if (next !== frame.value.view.zoom) void setView({ zoom: next })
}
function wheel(event: WheelEvent) { if (event.deltaY !== 0) zoom(event.deltaY < 0 ? 1 : -1) }
function drag(event: PointerEvent) {
  if (event.button !== 0 || !editable.value || annotating.value) return
  void api.dragPin().catch(e => { error.value = String(e) })
}
async function chooseTransform(value: string) {
  const item = transforms.find(item => item.action === value)
  if (item) await transform(item.action)
}
async function opacity(event: Event) {
  const input = event.target as HTMLInputElement
  await setView({ opacity: Number(input.value) / 100 })
  input.value = String(Math.round((frame.value?.view.opacity ?? 1) * 100))
}
function key(event: KeyboardEvent) {
  if (contextMenu.value) { handleEscape(event, () => closeMenu(true)); return }
  if (recognitionMode.value) {
    // 子面板处理过的 Esc 已 preventDefault；焦点意外落到外层时也只关面板。
    handleEscape(event, closeRecognition)
    return
  }
  if (handleEscape(event, () => {
    if (busy.value || editor.pending) return
    // 草稿从未保存；先放弃草稿解除交互锁，再走既有隐藏贴图接口。
    editor.abort(); void action('hide')
  })) return
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return
  if (annotating.value && editor.key(event)) return
  if ((event.target as HTMLElement).closest('input, textarea, select')) return
  if (event.repeat) return
  if (event.ctrlKey && event.altKey && event.key.toLowerCase() === 'p') {
    event.preventDefault(); closeMenu(); void setView({ clickThrough: !frame.value?.view.clickThrough }); return
  }
  const command = event.ctrlKey || event.metaKey
  const key = event.key.toLowerCase()
  if (command && ['c', 's', 'w'].includes(key)) {
    event.preventDefault(); void action(key === 'c' ? 'copy' : key === 's' ? 'save' : 'close'); return
  }
  if (key === 'escape') { event.preventDefault(); void action('hide'); return }
  if ((event.target as HTMLElement).closest('button, input, select')) return
  if (key === 'tab') { event.preventDefault(); toggleControls() }
  else if (key === '+' || key === '=') { event.preventDefault(); zoom(1) }
  else if (key === '-') { event.preventDefault(); zoom(-1) }
}
onMounted(async () => {
  window.addEventListener('pointerdown', dismissMenu)
  window.addEventListener('resize', closeOnResize)
  window.addEventListener('keydown', key)
  window.addEventListener('focus', updateFocus); window.addEventListener('blur', updateFocus)
  updateFocus()
  try {
    offVisibility = await onPinVisibility(visible => { void pinVisibility(visible) })
    if (disposed) { offVisibility(); return }
    offAppearance = await onPinAppearance(value => { appearanceRevision++; appearance.value = value })
    if (disposed) { offAppearance(); return }
    offView = await onPinView(view => { if (frame.value) frame.value.view = view })
    if (disposed) { offView(); return }
    await load()
  } catch (e) { if (!disposed) error.value = String(e) }
})
onUnmounted(() => {
  disposed = true; offView?.(); offAppearance?.(); offVisibility?.(); imageSession.dispose()
  clearTimeout(controlsHideTimer)
  window.removeEventListener('pointerdown', dismissMenu)
  window.removeEventListener('resize', closeOnResize)
  window.removeEventListener('keydown', key)
  window.removeEventListener('focus', updateFocus); window.removeEventListener('blur', updateFocus)
})
function closeOnResize() {
  viewport.value = { width: window.innerWidth, height: window.innerHeight, pixelRatio: window.devicePixelRatio }
  closeMenu()
}
</script>

<template>
  <div class="pin-shell" :class="{ 'is-focused': focused, 'controls-visible': controlsHovered || !!error || !!message }" :style="shellStyle" @pointerleave="hideControls" @contextmenu.prevent>
    <div v-show="imageReady" class="pin-stage" :inert="!!recognitionMode" tabindex="0" :aria-label='t("贴图，右键打开菜单")' :style="[stageStyle, pinAppearanceStyle(appearance, display.stageWidth, display.stageHeight, frame?.view.opacity ?? 1, focused)]" @pointerenter="keepControlsVisible" @pointerleave="scheduleControlsHide" @pointerdown="drag" @wheel.prevent="wheel" @contextmenu.capture="openMenu">
      <div v-if="frame" class="pin-canvas">
        <div class="pin-image-content" :style="{ width: `${display.imageWidth}px`, height: `${display.imageHeight}px`, opacity: frame.view.opacity }">
          <img v-if="frame.dataUrl" :src="frame.dataUrl" :alt='t("桌面贴图")' draggable="false" @load="imageLoaded" @error="error = '贴图预览加载失败，可关闭后重新贴图。'" />
          <AnnotationSurface :editor="editor" :width="frame.width" :height="frame.height" :bounds="bounds" :enabled="annotating && !busy && !frame.view.locked" />
        </div>
      </div>
    </div>
    <section ref="toolbar" :inert="!!recognitionMode" :style="floatingToolbarStyle" class="pin-controls" :class="{ collapsed: !controls, floating: floatingToolbarStyle.position }" :aria-label='t("贴图工具")' @pointerenter="keepControlsVisible" @pointerleave="scheduleControlsHide" @contextmenu.stop.prevent>
      <AnnotationToolbar v-if="frame && controls" :editor="editor" :disabled="busy || frame.view.locked" :active="annotating" @tool="startEditing" @float="floatToolbar" @reset-position="resetToolbarPosition">
        <template #leading><button :disabled="!editable" :aria-pressed="!annotating" :title='t("移动贴图")' :aria-label='t("移动贴图")' @click="annotating = false; editor.selected = -1"><Move :size="16" /></button></template>
        <template #actions>
          <button :disabled="busy" :aria-pressed="frame.view.topmost" :title='t("置顶开关")' :aria-label='t("置顶开关")' @click="setView({ topmost: !frame.view.topmost })"><component :is="frame.view.topmost ? Pin : PinOff" :size="16" /></button>
          <button :disabled="busy" :aria-pressed="frame.view.locked" :title="frame.view.locked ? t('解锁') : t('锁定')" :aria-label="frame.view.locked ? t('解锁') : t('锁定')" @click="setView({ locked: !frame.view.locked })"><component :is="frame.view.locked ? Lock : Unlock" :size="16" /></button>
          <span class="pin-divider" />
          <button :disabled="!editable || frame.view.zoom <= minZoom" :title='t("缩小")' :aria-label='t("缩小")' @click="zoom(-1)"><Minus :size="16" /></button>
          <button :disabled="!editable" :title='t("恢复 100% 查看比例")' @click="setView({ zoom: 1 })">{{ Math.round(frame.view.zoom * 100) }}%</button>
          <button :disabled="!editable" :title='t("放大")' :aria-label='t("放大")' @click="zoom(1)"><Plus :size="16" /></button>
          <span class="pin-divider" />
          <button :disabled="busy" :title='t("复制 Ctrl/⌘+C")' :aria-label='t("复制")' @click="action('copy')"><Copy :size="16" /></button>
          <button :disabled="busy" :title='t("保存图片 Ctrl/⌘+S")' :aria-label='t("保存图片")' @click="action('save')"><Save :size="16" /></button>
          <button :disabled="busy" :title='t("隐藏 Esc")' :aria-label='t("隐藏")' @click="action('hide')"><EyeOff :size="16" /></button>
          <button :disabled="busy" :title='t("关闭 Ctrl/⌘+W")' :aria-label='t("关闭")' @click="action('close')"><X :size="16" /></button>
          <button :disabled="busy" :title='t("收起工具（Tab）")' :aria-label='t("收起工具")' @click="toggleControls"><ChevronDown :size="16" /></button>
        </template>
        <template #options><div class="pin-row pin-options">
          <SelectControl model-value="" :aria-label="t('图片处理')" :disabled="!editable" :options="[{ value: '', label: t('图片处理') }, ...transforms.map(item => ({ value: item.action, label: t(item.label) }))]" @update:model-value="chooseTransform" @open-change="$event ? keepControlsVisible() : scheduleControlsHide()" />
          <label v-if="frame.transparencySupported" class="pin-opacity">{{ t("不透明度") }} <input type="range" min="20" max="100" step="5" :value="Math.round(frame.view.opacity * 100)" :disabled="busy" @change="opacity" /><span>{{ Math.round(frame.view.opacity * 100) }}%</span></label>
          <button :disabled="busy || editor.pending || editor.interacting" @click="recognize('ocr', $event)">{{ t("识别文字") }}</button>
          <button :disabled="busy || editor.pending || editor.interacting" @click="recognize('qr', $event)">{{ t("识别二维码") }}</button>
          <button :disabled="busy || editor.pending || editor.interacting" @click="recognize('translate', $event)">{{ t("翻译图片") }}</button>
          <span class="pin-dimensions">{{ frame.width }} × {{ frame.height }} px</span>
        </div></template>
      </AnnotationToolbar>
      <div v-else-if="!frame" class="pin-row"><span>{{ busy ? t("正在加载…") : t("图片未加载") }}</span><button :disabled="busy" @click="load">{{ t("重试") }}</button><button :disabled="busy" @click="action('close')">{{ t("关闭") }}</button></div>
      <button v-else class="pin-toggle" :disabled="busy" :title='t("展开贴图工具（右键）")' @click="toggleControls"><ChevronUp :size="16" />{{ t("工具") }}</button>
      <p v-if="error" role="alert" class="pin-status pin-error" :title="te(error)">{{ te(error) }}</p>
      <p v-else-if="message" role="status" class="pin-status" :title="t(message)">{{ t(message) }}</p>
      <p v-else-if="controls && !annotating" class="pin-status">{{ t("拖动移动 · 滚轮缩放 · 右键菜单 · Esc 隐藏") }}</p>
    </section>
    <p v-if="frame?.view.clickThrough" class="pin-pass-through" role="status">{{ t('鼠标穿透中 · 用全局贴图快捷键或托盘恢复') }}</p>
    <PinRecognition v-if="frame && recognitionMode" :frame="frame" :mode="recognitionMode" @close="closeRecognition" />
    <div v-if="contextMenu" ref="menuElement" class="pin-context-menu" role="menu" :aria-label='t("贴图菜单")' :style="{ left: `${contextMenu.x}px`, top: `${contextMenu.y}px` }" @pointerdown.stop @contextmenu.stop.prevent @keydown.stop="menuKey">
      <button role="menuitem" :disabled="busy || editor.pending || editor.interacting" @click="recognize('ocr', $event)">{{ t("识别文字") }}</button>
      <button role="menuitem" :disabled="busy || editor.pending || editor.interacting" @click="recognize('qr', $event)">{{ t("识别二维码") }}</button>
      <button role="menuitem" :disabled="busy || editor.pending || editor.interacting" @click="recognize('translate', $event)">{{ t("翻译图片") }}</button>
      <div role="separator" />
      <button role="menuitem" @click="closeMenu(true); action('copy')">{{ t("复制图片") }}<span>Ctrl/⌘+C</span></button>
      <button role="menuitem" @click="closeMenu(true); action('save')">{{ t("保存图片") }}<span>Ctrl/⌘+S</span></button>
      <div role="separator" />
      <button role="menuitemcheckbox" :aria-checked="frame?.view.topmost" :disabled="busy || editor.pending || editor.interacting" @click="closeMenu(true); setView({ topmost: !frame?.view.topmost })"><Check v-if="frame?.view.topmost" :size="14" class="menu-check" aria-hidden="true" />{{ t("窗口置顶") }}</button>
      <button role="menuitemcheckbox" :aria-checked="frame?.view.locked" :disabled="busy || editor.pending || editor.interacting" @click="closeMenu(true); setView({ locked: !frame?.view.locked })"><Check v-if="frame?.view.locked" :size="14" class="menu-check" aria-hidden="true" />{{ t("锁定贴图") }}</button>
      <button role="menuitemcheckbox" :aria-checked="controls" :disabled="busy || editor.pending || editor.interacting" @click="closeMenu(true); toggleControls()"><Check v-if="controls" :size="14" class="menu-check" aria-hidden="true" />{{ t("显示工具栏") }}<span>Tab</span></button>
      <button role="menuitemcheckbox" :aria-checked="!!frame?.view.clickThrough" :disabled="busy || editor.pending || editor.interacting" :title="t('用全局贴图快捷键或托盘恢复鼠标操作')" @click="closeMenu(); setView({ clickThrough: !frame?.view.clickThrough })"><Check v-if="frame?.view.clickThrough" :size="14" class="menu-check" aria-hidden="true" />{{ t("鼠标穿透") }}<span>Ctrl+Alt+P</span></button>
      <div role="separator" />
      <button role="menuitem" @click="closeMenu(true); action('hide')">{{ t("隐藏贴图") }}<span>Esc</span></button>
      <button role="menuitem" @click="closeMenu(true); action('close')">{{ t("关闭贴图") }}<span>Ctrl/⌘+W</span></button>
    </div>
  </div>
</template>

<style scoped>
.pin-pass-through { position: fixed; top: 4px; left: 8px; right: 8px; margin: 0; padding: 4px 8px; background: #fff; color: #24292f; border: 1px solid #d1d5db; font-size: 12px; pointer-events: none; z-index: 20; }
.pin-shell { position: fixed; inset: 0; display: grid; justify-items: center; color: #374151; user-select: none; }
.pin-stage { grid-row: 1; min-width: 0; min-height: 0; align-self: start; justify-self: start; overflow: auto; border-style: solid; border-color: #a1a1aa; transition: border-color .12s, box-shadow .12s, border-radius .12s; }
/* 缩放及窄窗口时只隐藏滚动条，保留原有滚动与键盘聚焦能力。 */
.pin-stage, .pin-controls, .pin-row, .pin-controls :deep(.annotation-tool-row) { scrollbar-width: none; }
.pin-stage::-webkit-scrollbar, .pin-controls::-webkit-scrollbar, .pin-row::-webkit-scrollbar, .pin-controls :deep(.annotation-tool-row::-webkit-scrollbar) { display: none; }
.pin-stage:focus-visible { outline:2px solid #3264df; outline-offset:2px; }
.pin-canvas { display: flex; align-items: center; justify-content: center; min-width: 100%; min-height: 100%; width: max-content; height: max-content; }
.pin-canvas img { flex: none; max-width: none; }
.pin-image-content { position: relative; flex: none; }
.pin-image-content img { display: block; width: 100%; height: 100%; }
.pin-controls { grid-row: 2; min-width: 0; width: max-content; max-width: 100%; height: 100%; justify-self: center; overflow: auto; padding: 4px 6px; border: 1px solid #d4d4d8; border-radius: 7px; background: #fffffff5; box-shadow: 0 2px 6px #0002; font: 13px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
/* 隐藏时保留底栏占位，避免图片跳动；键盘焦点仍可唤出工具。 */
.pin-shell:not(.controls-visible):not(:has(:focus-visible)) .pin-controls:not(.floating) { visibility: hidden; pointer-events: none; }
.pin-row { display: flex; align-items: center; gap: 3px; height: 34px; overflow-x: auto; overflow-y: hidden; white-space: nowrap; }
.pin-row > * { flex-shrink: 0; }
.pin-controls button { flex: 0 0 auto; display: inline-flex; align-items: center; justify-content: center; gap: 4px; height: 32px; min-width: 32px; padding: 3px 5px; border-radius: 4px; }
.pin-controls :deep(svg) { flex: 0 0 auto; }
.pin-controls button:hover { background: #eff6ff; }
.pin-controls button:disabled { opacity: .45; }
.pin-controls button[aria-pressed=true] { color: #2563eb; background: #dbeafe; }
.pin-controls button:focus-visible, .pin-controls select:focus-visible { outline: 2px solid #2563eb; outline-offset: -2px; }
.pin-divider { height: 14px; width: 1px; background: #e4e4e7; margin: 0 3px; }
.pin-options { gap: 8px; min-height: 36px; }
.pin-options select { background: transparent; max-width: 100px; height: 26px; }
.pin-opacity { display: flex; align-items: center; gap: 4px; }
.pin-opacity input { flex: 0 0 64px; width: 64px; accent-color: #2563eb; }
.pin-dimensions { color: #71717a; }
.pin-status { line-height: 16px; margin: 0; font-size: 10px; color: #71717a; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.pin-error { color: #b91c1c; }
.collapsed { display: flex; align-items: center; gap: 8px; }
.collapsed .pin-status { min-width: 0; }
.pin-toggle { flex-shrink: 0; }
.pin-context-menu { position: fixed; z-index: 20; width: 224px; max-width: calc(100vw - 8px); max-height: calc(100vh - 8px); overflow-y: auto; padding: 5px; border: 1px solid #e4e4e7; border-radius: 8px; background: #fff; box-shadow: 0 4px 18px #0003; font-size: 12px; }
.pin-context-menu .menu-check { position: absolute; left: 8px; top: 50%; transform: translateY(-50%); color: #2563eb; }
.pin-context-menu button { position: relative; display: flex; align-items: center; gap: 8px; width: 100%; min-height: 28px; padding: 4px 8px 4px 28px; border-radius: 4px; text-align: left; }
.pin-context-menu button:hover:not(:disabled), .pin-context-menu button:focus-visible { background: #eff6ff; color: #2563eb; outline: none; }
.pin-context-menu button:disabled { opacity: .4; }
.pin-context-menu button span { margin-left: auto; color: #a1a1aa; font-size: 10px; }
.pin-context-menu [role=separator] { height: 1px; background: #e4e4e7; margin: 4px; }
/* 透明窗口仅给工具区着色，保留图片周围的透明边距。 */
:global([data-theme=dark]) .pin-controls, :global([data-theme=dark]) .pin-context-menu { background: #20252c; color: #e6edf3; border-color: #39424e; }
:global([data-theme=dark]) .pin-controls button:hover:enabled, :global([data-theme=dark]) .pin-context-menu button:hover:enabled { background: #303f56; }
:global([data-theme=dark]) .pin-controls button[aria-pressed=true] { background: #303f56; color: #93c5fd; }
</style>
