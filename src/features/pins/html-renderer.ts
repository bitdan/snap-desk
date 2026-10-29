import { pinsApi } from './api'
import { onHtmlPinRender } from './events'
import { renderHtmlPin } from './html-pin'

/** 只在主窗口安装；即使主窗口隐藏，原生 F2 和托盘入口也能使用同一排版器。 */
export function installHtmlPinRenderer() {
  return onHtmlPinRender(request => {
    void (async () => {
      let png: string | null = null
      try { png = await renderHtmlPin(request.html) } catch { /* 由等待此请求的原生入口展示转换失败。 */ }
      await pinsApi.finishHtmlRender(request.id, png)
    })().catch(() => { /* 原生请求已超时，不记录或回传剪贴板正文。 */ })
  })
}
