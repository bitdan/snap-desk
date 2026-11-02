<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { t, te } from '@/shared/i18n/i18n'
import Button from '@/shared/ui/button/Button.vue'
import { handleEscape } from '@/shared/ui/escape'
import { useEscapeLayer } from '@/shared/ui/use-escape-layer'
import { onboardingSteps, tourLayout, type TourRect } from '@/app/onboarding'

const props = defineProps<{ busy: boolean; error?: string }>()
const emit = defineEmits<{ complete: []; navigate: [page: string] }>()
const step = ref(0), card = ref<HTMLElement | null>(null)
const portal = shallowRef<HTMLElement | null>(null), bounds = shallowRef<TourRect | null>(null)
const viewport = ref({ width: 1100, height: 680 }), cardHeight = ref(240)
const current = computed(() => onboardingSteps[step.value])
const layout = computed(() => tourLayout(bounds.value, viewport.value, cardHeight.value))
const px = (rect: TourRect) => ({ left: `${rect.left}px`, top: `${rect.top}px`, width: `${rect.width}px`, height: `${rect.height}px` })
const shades = computed(() => {
  const { width, height } = viewport.value, rect = layout.value.focus
  if (!rect) return [{ left: 0, top: 0, width, height }]
  const right = rect.left + rect.width, bottom = rect.top + rect.height
  return [
    { left: 0, top: 0, width, height: rect.top },
    { left: 0, top: bottom, width, height: height - bottom },
    { left: 0, top: rect.top, width: rect.left, height: rect.height },
    { left: right, top: rect.top, width: width - right, height: rect.height },
  ]
})
let target: HTMLElement | null = null, previousFocus: HTMLElement | null = null, targetStep = -1
let mutations: MutationObserver | undefined, sizes: ResizeObserver | undefined
let frame: number | undefined, disposed = false, focusPending = true
function complete() { if (!props.busy) emit('complete') }
function advance() {
  if (props.busy || !layout.value.focus) return
  if (step.value === onboardingSteps.length - 1) complete()
  else step.value++
}
function clicked() { if (targetStep === step.value && current.value.click) advance() }
function releaseTarget() { target?.removeEventListener('click', clicked); target = null; targetStep = -1 }
/** 页面切换可能异步挂载；观察目标出现后再定位，不依赖固定延时。 */
function locate() {
  if (disposed) return
  viewport.value = { width: window.innerWidth, height: window.innerHeight }
  const next = document.querySelector<HTMLElement>(`[data-onboarding="${current.value.target}"]`)
  if (next !== target) {
    releaseTarget(); target = next
    targetStep = step.value
    if (target) {
      target.scrollIntoView({ block: 'nearest', inline: 'nearest' })
      target.addEventListener('click', clicked)
    }
    sizes?.disconnect()
    if (target) sizes?.observe(target)
    if (card.value) sizes?.observe(card.value)
  }
  const rect = target?.getBoundingClientRect()
  const nextBounds = rect && rect.width > 0 && rect.height > 0 ? { left: rect.left, top: rect.top, width: rect.width, height: rect.height } : null
  // 忽略没有几何变化的观察通知，避免提示自身渲染再次触发定位循环。
  if (JSON.stringify(nextBounds) !== JSON.stringify(bounds.value)) bounds.value = nextBounds
  cardHeight.value = card.value?.getBoundingClientRect().height || 240
  if (focusPending && target) {
    focusPending = false
    if (current.value.click) target.focus({ preventScroll: true })
    else card.value?.focus({ preventScroll: true })
  }
}
function schedule() {
  if (frame !== undefined || disposed) return
  frame = requestAnimationFrame(() => { frame = undefined; locate() })
}
function key(event: KeyboardEvent) {
  if (handleEscape(event, complete) || event.key !== 'Tab' || event.defaultPrevented) return
  const selector = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]'
  const controls = [...(target?.matches(selector) ? [target] : []), ...(target?.querySelectorAll<HTMLElement>(selector) ?? []), ...(card.value?.querySelectorAll<HTMLElement>(selector) ?? [])]
  const first = controls[0], last = controls[controls.length - 1], active = document.activeElement
  if (first && last && (!controls.includes(active as HTMLElement) || (event.shiftKey ? active === first : active === last))) {
    event.preventDefault(); (event.shiftKey ? last : first).focus()
  }
}
useEscapeLayer(() => true, complete)
watch(step, async () => {
  releaseTarget(); bounds.value = null; focusPending = true
  emit('navigate', current.value.page)
  await nextTick(); schedule()
})
watch(() => props.error, () => { void nextTick(schedule) })
onMounted(() => {
  previousFocus = document.activeElement as HTMLElement | null
  portal.value = document.body
  emit('navigate', current.value.page)
  mutations = new MutationObserver(schedule)
  mutations.observe(document.querySelector('.app-shell') ?? document.body, { childList: true, subtree: true })
  if (typeof ResizeObserver !== 'undefined') sizes = new ResizeObserver(schedule)
  window.addEventListener('resize', schedule)
  window.addEventListener('scroll', schedule, true)
  window.addEventListener('keydown', key)
  void nextTick(schedule)
})
onUnmounted(() => {
  disposed = true; releaseTarget(); mutations?.disconnect(); sizes?.disconnect()
  if (frame !== undefined) cancelAnimationFrame(frame)
  window.removeEventListener('resize', schedule); window.removeEventListener('scroll', schedule, true); window.removeEventListener('keydown', key)
  if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true })
})
</script>

<template>
  <Teleport v-if="portal" :to="portal">
    <div class="onboarding-tour" :aria-label="t('新手引导')">
      <div v-for="(shade, index) in shades" :key="index" class="tour-shade" :style="px(shade)" aria-hidden="true" />
      <div v-if="layout.focus" class="tour-focus" :style="px(layout.focus)" aria-hidden="true" />
      <section ref="card" class="tour-card" role="dialog" aria-labelledby="tour-title" aria-describedby="tour-description" tabindex="-1" :aria-busy="busy" :style="{ left: `${layout.card.left}px`, top: `${layout.card.top}px`, width: `${layout.card.width}px` }">
        <header><h2 id="tour-title">{{ t(current.title) }}</h2><span>{{ step + 1 }} / {{ onboardingSteps.length }}</span></header>
        <p id="tour-description">{{ t(current.description) }}</p>
        <p v-if="!layout.focus" class="tour-wait" role="status">{{ t('正在定位操作区域…') }}</p>
        <p v-if="error" class="tour-error" role="alert">{{ te(error) }}</p>
        <footer><Button variant="ghost" :disabled="busy" @click="complete">{{ t('跳过引导') }}</Button><span class="tour-spacer" /><Button v-if="step > 0" variant="outline" :disabled="busy" @click="step--">{{ t('上一步') }}</Button><Button :disabled="busy || !layout.focus" @click="advance">{{ t(step === onboardingSteps.length - 1 ? '完成' : '下一步') }}</Button></footer>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.onboarding-tour { position:fixed; inset:0; z-index:500; pointer-events:none; font:13px 'Segoe UI','Microsoft YaHei',sans-serif; }
.tour-shade { position:absolute; background:#10182870; pointer-events:auto; }
.tour-focus { position:absolute; border:2px solid #2563eb; border-radius:6px; box-sizing:border-box; pointer-events:none; }
.tour-card { position:absolute; pointer-events:auto; max-height:calc(100vh - 24px); overflow:auto; box-sizing:border-box; padding:16px; border:1px solid var(--desk-border, #e1e4e8); border-radius:6px; background:var(--desk-bg, #fff); color:var(--desk-text, #24292f); box-shadow:0 4px 20px #0002; }
header, footer { display:flex; align-items:center; gap:8px; }
header h2 { flex:1; margin:0; font-size:14px; font-weight:600; }
header span, .tour-wait { color:var(--desk-muted, #57606a); font-size:12px; }
p { margin:12px 0; line-height:1.6; overflow-wrap:anywhere; }
footer { flex-wrap:wrap; margin-top:16px; }
.tour-spacer { flex:1; }
.tour-error { color:#b42318; }
.tour-card:focus-visible { outline:2px solid #2563eb; outline-offset:2px; }
:global([data-theme=dark]) .tour-error { color:#ff9c95; }
</style>
