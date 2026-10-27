import { api } from '@/services/api'
import { createThumbnailCache } from '@/shared/collections/thumbnail-cache'

// 仅 main WebView 的列表共用；不缓存原图，最多保留 64 个缩略图、约 8 MiB 字符串。
export const clipboardThumbnails = createThumbnailCache(id => api.clipboardThumbnail(id))
