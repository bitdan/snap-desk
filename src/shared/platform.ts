/** 当前 WebView 是否运行在 macOS；用于只影响展示的平台差异（标签、快捷键符号）。 */
export const macPlatform = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform || navigator.userAgent)
