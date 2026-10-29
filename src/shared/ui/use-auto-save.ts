import { onScopeDispose, ref } from 'vue'

/** 合并连续输入，串行提交；提交期间的新改动再保存一次，失败保留输入与可重试状态。 */
export function useAutoSave(save: () => Promise<void>, validate: () => string | null = () => null, delay = 350) {
  const saving = ref(false), error = ref(''), saved = ref(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  let pending = false
  let running: Promise<void> | undefined
  function schedule() {
    pending = true; saved.value = false; error.value = ''
    clearTimeout(timer)
    timer = setTimeout(() => { void flush() }, delay)
  }
  async function drain() {
    saving.value = true
    try {
      while (pending) {
        pending = false
        const invalid = validate()
        if (invalid) { error.value = invalid; saved.value = false; break }
        try { await save(); error.value = ''; saved.value = true }
        catch (e) { error.value = String(e); saved.value = false; break }
      }
    } finally { saving.value = false; running = undefined }
  }
  function flush(): Promise<void> {
    clearTimeout(timer); timer = undefined
    if (running) return running
    if (!pending) return Promise.resolve()
    // drain 可能同步结束（校验失败），通过微任务统一 running 的生命周期。
    running = Promise.resolve().then(drain)
    return running
  }
  function commit() { schedule(); return flush() }
  function retry() { pending = true; return flush() }
  onScopeDispose(() => { clearTimeout(timer); if (pending) void flush() })
  return { saving, error, saved, schedule, commit, flush, retry }
}
