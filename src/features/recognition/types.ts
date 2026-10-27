/** 与原生 camelCase 序列化保持一致的功能通信契约。 */

export interface QrResult { id: number; items: { text: string; url: string | null }[] }
