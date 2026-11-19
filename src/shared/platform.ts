/** 当前 WebView 是否运行在 macOS；用于只影响展示的平台差异（标签、快捷键符号）。 */
export const macPlatform = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform || navigator.userAgent)
/** 当前 WebView 是否运行在 Linux（WebKitGTK）；同样只用于展示差异，例如程序名示例不带 .exe。 */
export const linuxPlatform = typeof navigator !== 'undefined' && !macPlatform && /Linux/i.test(navigator.platform || navigator.userAgent) && !/Android/i.test(navigator.userAgent)
