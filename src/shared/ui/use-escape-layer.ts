import { watch } from 'vue'
import { registerEscapeLayer } from './escape'

/** 按打开顺序登记弹层；watch 清理同时覆盖关闭和组件卸载。 */
export function useEscapeLayer(active: () => boolean, close: () => void) {
  watch(active, (open, _, cleanup) => {
    if (open) cleanup(registerEscapeLayer(active, close))
  }, { immediate: true, flush: 'sync' })
}
