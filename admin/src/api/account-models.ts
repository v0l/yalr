import { API_BASE_URL } from './client'
import type { Model } from '../types'

export async function loadAccountModels(key: string): Promise<Model[]> {
  const response = await fetch(`${API_BASE_URL}/v1/models`, { headers: { Authorization: `Bearer ${key}` } })
  if (!response.ok) throw new Error(`Could not load models (${response.status})`)
  const body = await response.json()
  return body.data as Model[]
}

interface PublicModelEntry {
  id: string
  context_length: number
  architecture: { input_modalities: string[]; output_modalities: string[] }
  sats_pricing: { prompt: number; completion: number; request: number; input_cache_read: number; input_cache_write: number }
}

function perMillion(perToken: number): number {
  return Math.round(perToken * 1_000_000 * 1000) / 1000
}

export async function loadPublicModels(): Promise<Model[]> {
  const response = await fetch(`${API_BASE_URL}/v1/models`)
  if (!response.ok) throw new Error(`Could not load models (${response.status})`)
  const body = (await response.json()) as { data: PublicModelEntry[] }
  return body.data.map(m => ({
    id: m.id,
    object: 'model',
    created: 0,
    owned_by: '',
    context_length: m.context_length || undefined,
    architecture: m.architecture,
    pricing: {
      prompt: perMillion(m.sats_pricing.prompt),
      completion: perMillion(m.sats_pricing.completion),
      cache_read: perMillion(m.sats_pricing.input_cache_read),
      cache_write: perMillion(m.sats_pricing.input_cache_write),
      request: m.sats_pricing.request,
      unit: 'sats',
    },
  }))
}
