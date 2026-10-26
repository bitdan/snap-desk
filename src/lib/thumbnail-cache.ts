/** 缩略图使用有界 LRU 和并发队列；离开虚拟列表的请求从队列移除，在途响应不再写回缓存。 */
export function createThumbnailCache(loader: (id: number) => Promise<string>, maxChars = 4 * 1024 * 1024, maxEntries = 64, concurrency = 4) {
  const cache = new Map<number, string>()
  const queue: (() => void)[] = []
  let chars = 0, running = 0, generation = 0
  function pump() { while (running < concurrency && queue.length) queue.shift()!() }
  return {
    get(id: number, signal: AbortSignal): Promise<string> {
      if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'))
      const hit = cache.get(id)
      if (hit !== undefined) { cache.delete(id); cache.set(id, hit); return Promise.resolve(hit) }
      const version = generation
      return new Promise((resolve, reject) => {
        const abort = () => {
          const index = queue.indexOf(start)
          if (index >= 0) queue.splice(index, 1)
          reject(new DOMException('Aborted', 'AbortError'))
        }
        const start = () => {
          running++
          void Promise.resolve().then(() => loader(id)).then(value => {
            if (signal.aborted) return
            if (version !== generation) { resolve(value); return }
            const previous = cache.get(id)
            if (previous !== undefined) { chars -= previous.length; cache.delete(id) }
            if (value.length <= maxChars) { cache.set(id, value); chars += value.length }
            while (chars > maxChars || cache.size > maxEntries) {
              const oldest = cache.keys().next().value!
              chars -= cache.get(oldest)!.length; cache.delete(oldest)
            }
            resolve(value)
          }, reject).finally(() => { signal.removeEventListener('abort', abort); running--; pump() })
        }
        signal.addEventListener('abort', abort, { once: true })
        queue.push(start); pump()
      })
    },
    /** 删除历史/退出页面时可主动释放缓存，避免已删除的图像数据长期驻留。 */
    clear() { generation++; cache.clear(); chars = 0 },
  }
}
