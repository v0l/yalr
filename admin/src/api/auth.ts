import { useEffect, useState } from 'react'
import { API_BASE_URL } from './client'

export interface Session {
  token: string
  username: string
  displayName?: string
  isAdmin: boolean
}

interface SessionResponse {
  token: string
  username: string
  display_name?: string
  is_admin: boolean
}

interface NostrSigner {
  getPublicKey(): Promise<string>
  signEvent(event: { kind: number; created_at: number; tags: string[][]; content: string }): Promise<unknown>
}

declare global {
  interface Window {
    nostr?: NostrSigner
  }
}

function toSession(r: SessionResponse): Session {
  return { token: r.token, username: r.username, displayName: r.display_name, isAdmin: r.is_admin }
}

export function storeSession(session: Session) {
  localStorage.setItem('token', session.token)
  localStorage.setItem('user', JSON.stringify({ username: session.username, displayName: session.displayName, isAdmin: session.isAdmin }))
}

async function post(path: string, init: RequestInit): Promise<Session> {
  const response = await fetch(`${API_BASE_URL}${path}`, { method: 'POST', ...init })
  if (!response.ok) {
    const text = await response.text().catch(() => '')
    throw new Error(text || `Request failed (${response.status})`)
  }
  return toSession(await response.json())
}

export function useNostrLogin(): boolean {
  const [enabled, setEnabled] = useState(false)
  useEffect(() => {
    fetch(`${API_BASE_URL}/api/setup/status`)
      .then(r => (r.ok ? r.json() : null))
      .then((s: { nostr_login?: boolean } | null) => setEnabled(!!s?.nostr_login))
      .catch(() => setEnabled(false))
  }, [])
  return enabled
}

export const authApi = {
  login: (username: string, password: string) =>
    post('/api/auth/login', { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username, password }) }),
  register: (username: string, password: string) =>
    post('/api/auth/register', { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username, password }) }),
  hasNostr: () => typeof window !== 'undefined' && !!window.nostr,
  async nostr(): Promise<Session> {
    if (!window.nostr) throw new Error('No Nostr extension found. Install a NIP-07 signer such as Alby or nos2x.')
    const url = new URL('/api/auth/nostr', API_BASE_URL || window.location.origin).toString()
    const event = await window.nostr.signEvent({
      kind: 27235,
      created_at: Math.floor(Date.now() / 1000),
      tags: [['u', url], ['method', 'POST']],
      content: '',
    })
    return post('/api/auth/nostr', { headers: { Authorization: `Nostr ${btoa(JSON.stringify(event))}` } })
  },
}
