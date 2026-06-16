import { useCallback, useRef, useState } from 'react'
import { api } from '../../api/client'

export interface ProviderModel { id: string; created: number; owned_by: string }

interface ModelEntry { models: ProviderModel[]; loading: boolean; error: boolean }

/**
 * Lazily loads and caches the model list for each provider slug so inline
 * model editors across rows share a single fetch per provider.
 */
export function useModelCache() {
  const [cache, setCache] = useState<Record<string, ModelEntry>>({})
  const inflight = useRef<Set<string>>(new Set())

  const ensure = useCallback((slug: string) => {
    if (!slug || inflight.current.has(slug) || cache[slug]?.models.length) return
    inflight.current.add(slug)
    setCache(prev => ({ ...prev, [slug]: { models: [], loading: true, error: false } }))
    api
      .getProviderModels(slug)
      .then(d => setCache(prev => ({ ...prev, [slug]: { models: d.models, loading: false, error: false } })))
      .catch(() => setCache(prev => ({ ...prev, [slug]: { models: [], loading: false, error: true } })))
      .finally(() => inflight.current.delete(slug))
  }, [cache])

  const get = useCallback(
    (slug: string): ModelEntry => cache[slug] ?? { models: [], loading: false, error: false },
    [cache],
  )

  return { ensure, get }
}
