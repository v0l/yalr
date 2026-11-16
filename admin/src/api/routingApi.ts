import { API_BASE_URL } from './client'

export type RoutingPreference = 'balanced' | 'cheapest' | 'fastest'

export interface RoutingSettings {
  default: RoutingPreference
  models: { model: string; preference: RoutingPreference }[]
}

async function call(method: 'GET' | 'PUT', body?: RoutingSettings): Promise<RoutingSettings> {
  const token = localStorage.getItem('token')
  const response = await fetch(`${API_BASE_URL}/api/account/routing`, {
    method,
    headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  })
  if (!response.ok) throw new Error((await response.text().catch(() => '')) || `Request failed (${response.status})`)
  return response.json()
}

export const routingApi = {
  load: () => call('GET'),
  save: (settings: RoutingSettings) => call('PUT', settings),
}
