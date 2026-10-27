/**
 * 给冷启动 WebView 留出样式、布局和图片合成的机会。
 * 隐藏窗口的 rAF 可能被系统暂停，所以定时器兜底，避免截图一直无法显示。
 */
export function waitForCapturePaint(): Promise<void> {
  return new Promise(resolve => {
    let first = 0, second = 0
    const finish = () => {
      clearTimeout(timer)
      cancelAnimationFrame(first)
      cancelAnimationFrame(second)
      resolve()
    }
    const timer = setTimeout(finish, 100)
    first = requestAnimationFrame(() => { second = requestAnimationFrame(finish) })
  })
}
