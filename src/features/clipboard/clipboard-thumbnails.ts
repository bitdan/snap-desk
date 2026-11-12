import { api } from '@/services/api'
import { createThumbnailCache } from '@/shared/collections/thumbnail-cache'

// 每个 WebView 各有一份：历史列表与暂存架卡片分别复用；不缓存原图，最多保留 64 个缩略图、约 8 MiB 字符串。
export const clipboardThumbnails = createThumbnailCache(id => api.clipboardThumbnail(id))
