<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { MAGNIFIER_SIZE, MAGNIFIER_ZOOM, magnifierPosition, magnifierSample } from '@/lib/magnifier'
import type { Point, Size } from '@/lib/selection'
import { colorLabels } from '@/lib/color-picker'

const props = defineProps<{ image: HTMLImageElement; point: Point; viewport: Size; imageSize: Size; showColor?: boolean }>()
const color = ref<{ hex: string; rgb: string } | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
const sample = computed(() => magnifierSample(props.point, props.viewport, props.imageSize))
const position = computed(() => magnifierPosition(props.point, props.viewport, { width: 122, height: props.showColor ? 194 : 150 }))
let animation = 0
// 复用蒙层已经解码的截图，只绘制 15 × 15 原图像素；高频移动每帧最多刷新一次。
watch([sample, () => props.image, () => props.showColor, canvas], () => {
  cancelAnimationFrame(animation)
  animation = requestAnimationFrame(() => {
    const context = canvas.value?.getContext('2d')
    if (!context || !props.image.complete || !props.image.naturalWidth) return
    const s = sample.value
    context.clearRect(0, 0, MAGNIFIER_SIZE, MAGNIFIER_SIZE)
    context.imageSmoothingEnabled = false
    context.drawImage(props.image, s.x, s.y, s.width, s.height, s.dx, s.dy,
      s.width * MAGNIFIER_ZOOM, s.height * MAGNIFIER_ZOOM)
    if (props.showColor) {
      const pixel = context.getImageData(60, 60, 1, 1).data
      color.value = colorLabels(pixel[0], pixel[1], pixel[2])
    }
  })
}, { immediate: true, flush: 'post' })
onUnmounted(() => cancelAnimationFrame(animation))
</script>

<template>
  <div class="capture-magnifier" :style="{ left: `${position.x}px`, top: `${position.y}px`, height: showColor ? '194px' : '150px' }" aria-hidden="true">
    <div class="magnifier-preview">
      <canvas ref="canvas" :width="MAGNIFIER_SIZE" :height="MAGNIFIER_SIZE" />
      <span class="magnifier-cross horizontal" /><span class="magnifier-cross vertical" />
      <span class="magnifier-pixel" />
    </div>
    <div class="magnifier-coordinate">{{ sample.pixel.x }}, {{ sample.pixel.y }} · 8×</div>
    <div v-if="showColor && color" class="magnifier-color"><span><i :style="{ background: color.hex }" />{{ color.hex }}</span><span>{{ color.rgb }}</span></div>
  </div>
</template>

<style scoped>
.capture-magnifier { position: absolute; z-index: 3; pointer-events: none; width: 122px; height: 150px; border: 1px solid #a1a1aa; background: #18181b; color: white; box-shadow: 0 3px 12px #0006; overflow: hidden; }
.magnifier-preview { position: relative; width: 120px; height: 120px; background: #52525b; }
canvas { display: block; width: 120px; height: 120px; image-rendering: pixelated; }
.magnifier-cross { position: absolute; background: #fff9; box-shadow: 0 0 1px #000; }
.horizontal { left: 0; top: 59px; width: 120px; height: 1px; }
.vertical { top: 0; left: 59px; height: 120px; width: 1px; }
.magnifier-pixel { position: absolute; top: 56px; left: 56px; width: 8px; height: 8px; border: 1px solid #ef4444; box-shadow: 0 0 0 1px #fff; }
.magnifier-coordinate { height: 28px; line-height: 28px; text-align: center; font-size: 11px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.magnifier-color { font-size: 10px; text-align: center; line-height: 20px; font-variant-numeric: tabular-nums; }
.magnifier-color span { display: block; white-space: nowrap; }
.magnifier-color i { display: inline-block; width: 10px; height: 10px; border: 1px solid white; margin-right: 5px; }
</style>
