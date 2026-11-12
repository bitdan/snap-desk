import messages from '@/shared/i18n/messages.en.json'
import nativeMessages from '@/shared/i18n/messages.native.en.json'
import slots from '@/shared/i18n/message-error-slots.json'

/**
 * 英文词典单独成块：只有界面语言为英文时才由 i18n.ts 动态导入，
 * 中文界面下的贴图、选区边框、录屏控制条等小窗口不必下载和解析它。
 * 前后端共享原生消息表；界面词条覆盖同名的原生词条。
 */
export const english: Record<string, string> = { ...nativeMessages, ...messages }

/** 模板中允许递归翻译的参数序号；其余参数（路径、识别文字等）始终原样插入。 */
export const errorSlots: Record<string, number[]> = slots
