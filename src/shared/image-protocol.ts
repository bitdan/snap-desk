import { convertFileSrc } from '@tauri-apps/api/core'

/**
 * 原生只读图片协议：路径只含贴图标签、历史 ID 与像素版本，不接受文件路径；
 * 读取权限由 Rust 按发起请求的窗口校验，WebView 直接解码二进制 PNG，不经过 Base64 与 JSON。
 */
const route = (name: string) => convertFileSrc(name, 'snap-image')
export const pinImageUrl = (label: string, revision: number) => `${route('pin')}/${label}/${revision}.png`
export const clipboardImageUrl = (id: number) => `${route('clip')}/${id}.png`
