import type { ThreadMessage, ThreadMessageLike } from '@assistant-ui/react'

const KEY = 'chat-history'

export function loadChatHistory(): ThreadMessageLike[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    if (!Array.isArray(parsed)) return []
    return parsed.map(m => ({ ...m, createdAt: m.createdAt ? new Date(m.createdAt) : undefined }))
  } catch {
    return []
  }
}

export function saveChatHistory(messages: readonly ThreadMessage[]) {
  try {
    if (messages.length === 0) localStorage.removeItem(KEY)
    else localStorage.setItem(KEY, JSON.stringify(messages))
  } catch {
    localStorage.removeItem(KEY)
  }
}
