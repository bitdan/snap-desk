<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { MAGNIFIER_SIZE, MAGNIFIER_ZOOM, magnifierPosition, magnifierSample } from '@/lib/magnifier'
import type { Point, Size, Rect } from '@/lib/selection'
import { colorLabels } from '@/lib/color-picker'
import type { ColorFormat } from '@/lib/api'

const props = defineProps<{ image: HTMLImageElement; point: Point; viewport: Size; imageSize: Size; format: ColorFormat; relative: boolean; origin: Point | null; copied: boolean; bounds?: Rect }>()
const color = ref<{ hex: string; rgb: string } | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
const sample = computed(() => magnifierSample(props.point, props.viewport, props.imageSize))
const position = computed(() => {
  const bounds = props.bounds ?? { x: 0, y: 0, ...props.viewport }
  const local = magnifierPosition({ x: props.point.x - bounds.x, y: props.point.y - bounds.y }, bounds, { width: 167, height: 212 })
  return { x: local.x + bounds.x, y: local.y + bounds.y }
})
// 全局坐标以当前截图左上角为原点；相对坐标以选区左上角为原点，未框选时两者一致。
const coordinate = computed(() => ({
  x: sample.value.pixel.x - (props.relative ? props.origin?.x ?? 0 : 0),
  y: sample.value.pixel.y - (props.relative ? props.origin?.y ?? 0 : 0),
}))
let animation = 0
// 复用蒙层已经解码的截图，只绘制 15 × 15 原图像素；高频移动每帧最多刷新一次。
watch([sample, () => props.image, canvas], () => {
  cancelAnimationFrame(animation)
  animation = requestAnimationFrame(() => {
    const context = canvas.value?.getContext('2d')
    if (!context || !props.image.complete || !props.image.naturalWidth) return
    const s = sample.value
    context.clearRect(0, 0, MAGNIFIER_SIZE, MAGNIFIER_SIZE)
    context.imageSmoothingEnabled = false
    context.drawImage(props.image, s.x, s.y, s.width, s.height, s.dx, s.dy,
      s.width * MAGNIFIER_ZOOM, s.height * MAGNIFIER_ZOOM)
    // 网格、准星通过 CSS 叠加，不污染用于色值读取的原始像素。
    const pixel = context.getImageData(MAGNIFIER_SIZE / 2, MAGNIFIER_SIZE / 2, 1, 1).data
    color.value = colorLabels(pixel[0], pixel[1], pixel[2])
  })
}, { immediate: true, flush: 'post' })
onUnmounted(() => cancelAnimationFrame(animation))
</script>

<template>
  <div class="capture-magnifier" :style="{ left: `${position.x}px`, top: `${position.y}px` }" aria-hidden="true">
    <div class="magnifier-preview">
      <canvas ref="canvas" :width="MAGNIFIER_SIZE" :height="MAGNIFIER_SIZE" />
      <span class="magnifier-grid" />
      <span class="magnifier-cross horizontal" /><span class="magnifier-cross vertical" />
      <span class="magnifier-pixel" :style="{ background: color?.hex }" />
    </div>
    <div class="magnifier-coordinate">{{ relative ? '相对 ' : '' }}({{ coordinate.x }}, {{ coordinate.y }})</div>
    <div class="magnifier-color"><template v-if="color"><i :style="{ background: color.hex }" />{{ color[format] }}</template></div>
    <div class="magnifier-hint">Shift: 切换颜色格式</div>
    <div class="magnifier-hint" :class="{ copied }">{{ copied ? '已复制，可继续截图' : 'C: 复制颜色值' }}</div>
    <div class="magnifier-hint">P: 切换全局/相对坐标</div>
  </div>
</template>

<style scoped>
.capture-magnifier { position: absolute; z-index: 3; pointer-events: none; box-sizing: border-box; width: 167px; height: 212px; border: 1px solid #222; background: #242424; color: white; overflow: hidden; }
/* 每个原图像素显示为 11px，截取中间九行，中央像素仍与采样坐标严格对应。 */
.magnifier-preview { position: relative; width: 165px; height: 99px; overflow: hidden; background: #52525b; }
canvas { position: absolute; top: -33px; left: 0; width: 165px; height: 165px; image-rendering: pixelated; }
.magnifier-grid { position: absolute; inset: 0; background-image: linear-gradient(to right, #0004 1px, transparent 1px), linear-gradient(to bottom, #0004 1px, transparent 1px); background-size: 11px 11px; }
.magnifier-cross { position: absolute; background: #3b82f680; }
.horizontal { left: 0; top: 44px; width: 165px; height: 11px; }
.vertical { top: 0; left: 77px; height: 99px; width: 11px; }
.magnifier-pixel { position: absolute; top: 44px; left: 77px; width: 11px; height: 11px; box-sizing: border-box; border: 1px solid #111; box-shadow: 0 0 0 1px #fff; }
.magnifier-coordinate, .magnifier-color { height: 23px; line-height: 23px; text-align: center; font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.magnifier-color i { display: inline-block; vertical-align: -2px; width: 13px; height: 13px; border: 1px solid white; margin-right: 7px; }
.magnifier-hint { font-size: 12px; line-height: 21px; text-align: center; white-space: nowrap; }
.copied { color: #86efac; }
</style>
