import { nextTick } from 'vue'
import type { CapturePaint } from './types'

/**
 * 给冷启动 WebView 留出样式、布局和图片合成的机会。
 * 隐藏窗口的 rAF 可能被系统暂停，所以定时器兜底，避免截图一直无法显示。
 */
export function waitForCapturePaint(): Promise<{ elapsedMs: number; source: 'raf' | 'timeout' }> {
  return new Promise(resolve => {
    const started = performance.now()
    let first = 0, second = 0
    let settled = false
    const finish = (source: 'raf' | 'timeout') => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      cancelAnimationFrame(first)
      cancelAnimationFrame(second)
      resolve({ elapsedMs: performance.now() - started, source })
    }
    const timer = setTimeout(() => finish('timeout'), 100)
    first = requestAnimationFrame(() => { second = requestAnimationFrame(() => finish('raf')) })
  })
}

/** 图片解码 → 隐藏窗口预合成 → 原生显示 → 可见绘制回执；每次异步恢复均核对图片身份。 */
export async function presentCaptureImage(options: {
  image: Pick<HTMLImageElement, 'decode'>; receivedAt: number; isCurrent: () => boolean;
  decoded: () => void; decodeFailed: () => Promise<void>; ready: () => Promise<void>;
  painted: (paint: CapturePaint) => Promise<void>;
}): Promise<void> {
  const loadMs = Math.max(0, performance.now() - options.receivedAt)
  const decodeStarted = performance.now()
  try { await options.image.decode() }
  catch { if (options.isCurrent()) await options.decodeFailed(); return }
  const decodeMs = performance.now() - decodeStarted
  if (!options.isCurrent()) return
  options.decoded()
  await nextTick()
  const pre = await waitForCapturePaint()
  if (!options.isCurrent()) return
  await options.ready()
  if (!options.isCurrent()) return
  // 用户此时已可操作；此处只补测量，不将回执放进显示窗口的关键路径。
  const post = await waitForCapturePaint()
  if (!options.isCurrent()) return
  try {
    await options.painted({ loadMs, decodeMs, prePaintMs: pre.elapsedMs, postPaintMs: post.elapsedMs, paintSource: post.source })
  } catch { /* 诊断回执失败不影响已经显示的截图。 */ }
}
