import { API_BASE_URL } from './client'
import type { Model } from '../types'

export async function loadAccountModels(key: string): Promise<Model[]> {
  const response = await fetch(`${API_BASE_URL}/v1/models`, { headers: { Authorization: `Bearer ${key}` } })
  if (!response.ok) throw new Error(`Could not load models (${response.status})`)
  const body = await response.json()
  return body.data as Model[]
}
