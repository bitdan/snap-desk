/** 原文由原生命令一次性领取，通知本身不携带用户文字。 */
import { listen } from '@/services/ipc'
export const onTranslationOpen = (fn: () => void) => listen('translation-open', fn)
