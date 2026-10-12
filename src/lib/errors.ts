/** Tauri 的传输层错误可能是二进制响应；直接 String() 会丢失实际错误正文。 */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  if (error instanceof ArrayBuffer || ArrayBuffer.isView(error)) {
    const bytes = error instanceof ArrayBuffer
      ? new Uint8Array(error)
      : new Uint8Array(error.buffer, error.byteOffset, error.byteLength)
    const text = new TextDecoder().decode(bytes).trim()
    if (!text) return '桌面通信返回了空错误响应，请退出应用后重试。'
    // 某些运行环境把 JSON 错误也以二进制传回，去掉 JSON 字符串的引号。
    try { return errorMessage(JSON.parse(text)) } catch { return text }
  }
  if (typeof error === 'string') return error
  if (error && typeof error === 'object') {
    if ('message' in error && typeof error.message === 'string') return error.message
    try { return JSON.stringify(error) } catch { /* 循环对象回退到通用提示。 */ }
  }
  return error == null ? '桌面通信失败，未返回错误详情。' : String(error)
}
