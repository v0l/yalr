import { useState, useMemo } from 'react'
import { SearchIcon, XIcon, ChevronDownIcon, SparklesIcon, PlusIcon, ExternalLinkIcon } from 'lucide-react'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import type { ProviderFormData } from '../types'

/* ── Provider catalog ────────────────────────────────────────────── */

interface CatalogProvider {
  name: string
  slug: string
  provider_type: string
  base_url: string
  description: string
  models: string
  api_keys_url: string
  category: 'major' | 'opensource_platform' | 'routing'
}

const CATALOG: CatalogProvider[] = [
  {
    name: 'OpenAI',
    slug: 'openai',
    provider_type: 'openai',
    base_url: 'https://api.openai.com/v1',
    description: 'GPT-4o, o3, o4-mini, and DALL·E. The industry standard for general-purpose AI.',
    models: 'GPT-4o · GPT-4.1 · o3 · o4-mini · DALL·E 3',
    api_keys_url: 'https://platform.openai.com/api-keys',
    category: 'major',
  },
  {
    name: 'Anthropic',
    slug: 'anthropic',
    provider_type: 'anthropic',
    base_url: 'https://api.anthropic.com',
    description: 'Claude Opus, Sonnet, and Haiku. Best-in-class for coding and thoughtful analysis.',
    models: 'Claude Opus 4 · Claude Sonnet 4 · Claude Haiku',
    api_keys_url: 'https://console.anthropic.com/settings/keys',
    category: 'major',
  },
  {
    name: 'Google Gemini',
    slug: 'google',
    provider_type: 'openai',
    base_url: 'https://generativelanguage.googleapis.com/v1beta/openai',
    description: 'Gemini 2.5 Pro & Flash. Massive context windows and multimodal prowess.',
    models: 'Gemini 2.5 Pro · Gemini 2.5 Flash · Gemma',
    api_keys_url: 'https://aistudio.google.com/apikey',
    category: 'major',
  },
  {
    name: 'Meta Llama',
    slug: 'meta',
    provider_type: 'openai',
    base_url: 'https://api.llama-api.com',
    description: 'Llama 4 Scout & Maverick. Open-weight models from Meta, strong all-rounders.',
    models: 'Llama 4 Scout · Llama 4 Maverick · Llama 3.3 70B',
    api_keys_url: 'https://www.llama-api.com',
    category: 'major',
  },
  {
    name: 'xAI Grok',
    slug: 'xai',
    provider_type: 'openai',
    base_url: 'https://api.x.ai/v1',
    description: 'Grok by xAI. Fast, unfiltered models with real-time X access.',
    models: 'Grok 3 · Grok 3 Mini',
    api_keys_url: 'https://console.x.ai',
    category: 'major',
  },
  {
    name: 'Mistral AI',
    slug: 'mistral',
    provider_type: 'openai',
    base_url: 'https://api.mistral.ai/v1',
    description: 'Mistral Large & Small. Elegant, efficient European models with strong multilingual support.',
    models: 'Mistral Large 2 · Mistral Small · Codestral · Pixtral',
    api_keys_url: 'https://console.mistral.ai/api-keys',
    category: 'major',
  },
  {
    name: 'Cohere',
    slug: 'cohere',
    provider_type: 'openai',
    base_url: 'https://api.cohere.ai/v1',
    description: 'Command R & Embed. Enterprise-grade RAG, embeddings, and multilingual models.',
    models: 'Command R+ · Command R · Embed v4 · Rerank',
    api_keys_url: 'https://dashboard.cohere.com/api-keys',
    category: 'major',
  },
  {
    name: 'DeepSeek',
    slug: 'deepseek',
    provider_type: 'openai',
    base_url: 'https://api.deepseek.com/v1',
    description: 'DeepSeek-V3 & R1. Exceptional reasoning models at unbeatable prices.',
    models: 'DeepSeek-V3 · DeepSeek-R1 · DeepSeek-Coder',
    api_keys_url: 'https://platform.deepseek.com/api_keys',
    category: 'major',
  },
  {
    name: 'Qwen (Alibaba)',
    slug: 'qwen',
    provider_type: 'openai',
    base_url: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1',
    description: 'Alibaba\'s Qwen 3 family. Top-tier multilingual models, especially strong in Chinese & coding.',
    models: 'Qwen 3 Max · Qwen 3 Coder · Qwen 2.5 VL',
    api_keys_url: 'https://bailian.console.aliyun.com/?apiKey=1',
    category: 'major',
  },
  {
    name: 'Zhipu AI (GLM)',
    slug: 'zhipu',
    provider_type: 'openai',
    base_url: 'https://open.bigmodel.cn/api/paas/v4',
    description: 'GLM-4 Plus by Zhipu AI. Leading Chinese AI lab, strong multimodal & agent capabilities.',
    models: 'GLM-4 Plus · GLM-4 Flash · CogView-4 · CogVideoX',
    api_keys_url: 'https://open.bigmodel.cn/usercenter/apikeys',
    category: 'major',
  },
  {
    name: 'Moonshot AI (Kimi)',
    slug: 'moonshot',
    provider_type: 'openai',
    base_url: 'https://api.moonshot.cn/v1',
    description: 'Creators of Kimi. Ultra-long context (128K+) models popular across Asia.',
    models: 'Kimi K2 · Kimi K1.5 · Moonshot-v1',
    api_keys_url: 'https://platform.moonshot.cn/console/api-keys',
    category: 'major',
  },
  {
    name: 'Z.ai',
    slug: 'zai',
    provider_type: 'openai',
    base_url: 'https://api.z.ai/api/v1',
    description: 'GLM & CodeGeeX inference platform. Affordable access to Chinese LLMs via OpenAI-compatible API.',
    models: 'GLM-4 · CodeGeeX-4 · ChatGLM · CogView',
    api_keys_url: 'https://api.z.ai/api/v1',
    category: 'major',
  },
  {
    name: 'Groq',
    slug: 'groq',
    provider_type: 'openai',
    base_url: 'https://api.groq.com/openai/v1',
    description: 'Blazing fast open-source model inference on custom LPU hardware. Free tier available.',
    models: 'Llama 4 · Mixtral · DeepSeek · Gemma · Qwen',
    api_keys_url: 'https://console.groq.com/keys',
    category: 'opensource_platform',
  },
  {
    name: 'Together AI',
    slug: 'together',
    provider_type: 'openai',
    base_url: 'https://api.together.xyz/v1',
    description: 'Fast inference for 200+ open models. Great for fine-tuning and custom deployments.',
    models: 'Llama 4 · DeepSeek-V3 · Qwen 3 · Mixtral · Yi',
    api_keys_url: 'https://api.together.xyz/settings/api-keys',
    category: 'opensource_platform',
  },
  {
    name: 'OpenRouter',
    slug: 'openrouter',
    provider_type: 'openrouter',
    base_url: 'https://openrouter.ai/api/v1',
    description: 'Unified API for 300+ models from every provider. Pay once, route anywhere.',
    models: 'OpenAI · Anthropic · Google · DeepSeek · Meta · 300+ more',
    api_keys_url: 'https://openrouter.ai/keys',
    category: 'routing',
  },
  {
    name: 'PPQ.ai',
    slug: 'ppq',
    provider_type: 'ppq',
    base_url: 'https://api.ppq.ai',
    description: 'Privacy-first AI inference paid with Bitcoin Lightning. No accounts, no tracking.',
    models: 'ChatGPT · Claude · Gemini · DeepSeek · Llama',
    api_keys_url: 'https://api.ppq.ai',
    category: 'routing',
  },
]

/* ── Category config ──────────────────────────────────────────────── */

const CATEGORIES = [
  { key: 'all', label: 'All Providers' },
  { key: 'major', label: 'AI Labs & Models' },
  { key: 'opensource_platform', label: 'Inference Platforms' },
  { key: 'routing', label: 'Routing Services' },
] as const

type CategoryKey = (typeof CATEGORIES)[number]['key']

/* ── Props ────────────────────────────────────────────────────────── */

export interface ProviderCatalogDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSelect: (prefill: Partial<ProviderFormData>, apiKeysUrl?: string) => void
  onCustomAdd: () => void
}

/* ── Component ────────────────────────────────────────────────────── */

export function ProviderCatalogDialog({
  open,
  onOpenChange,
  onSelect,
  onCustomAdd,
}: ProviderCatalogDialogProps) {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState<CategoryKey>('all')
  const [expanded, setExpanded] = useState(false)

  const filtered = useMemo(() => {
    let list = CATALOG
    if (category !== 'all') list = list.filter(p => p.category === category)
    if (search.trim()) {
      const q = search.toLowerCase()
      list = list.filter(
        p =>
          p.name.toLowerCase().includes(q) ||
          p.description.toLowerCase().includes(q) ||
          p.models.toLowerCase().includes(q) ||
          p.slug.includes(q),
      )
    }
    return list
  }, [search, category])

  const visibleProviders = expanded ? filtered : filtered.slice(0, 8)

  function handleSelect(prefill: Partial<ProviderFormData>, apiKeysUrl?: string) {
    onSelect(prefill, apiKeysUrl)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-3xl max-h-[85vh] overflow-hidden flex flex-col border-border bg-card">
        <DialogHeader className="shrink-0">
          <DialogTitle className="font-display text-xl tracking-[0.04em] flex items-center gap-2">
            <SparklesIcon className="size-4 text-brand" />
            ADD PROVIDER
          </DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            Pick a provider to get started with pre-filled settings, or add a custom one.
          </DialogDescription>
        </DialogHeader>

        {/* Search + categories */}
        <div className="shrink-0 flex flex-col sm:flex-row gap-2 mt-2">
          <div className="relative flex-1">
            <SearchIcon className="absolute left-2.5 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground pointer-events-none" />
            <input
              type="text"
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder="Search by name, model, or feature..."
              className="w-full bg-surface border border-border font-mono text-[13px] text-foreground placeholder:text-muted-foreground/60 pl-8 pr-8 py-2 focus:outline-none focus:border-brand/50 transition-colors"
            />
            {search && (
              <button
                onClick={() => setSearch('')}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors"
              >
                <XIcon className="size-3.5" />
              </button>
            )}
          </div>
          <div className="flex gap-1 flex-wrap">
            {CATEGORIES.map(cat => (
              <button
                key={cat.key}
                onClick={() => setCategory(cat.key)}
                className={`font-mono text-[11px] px-2.5 py-1.5 border transition-colors cursor-pointer whitespace-nowrap ${
                  category === cat.key
                    ? 'border-brand/40 bg-brand/10 text-brand'
                    : 'border-border text-muted-foreground hover:text-foreground hover:border-muted-foreground/30'
                }`}
              >
                {cat.label}
              </button>
            ))}
          </div>
        </div>

        {/* Provider grid — scrollable */}
        <div className="flex-1 overflow-y-auto mt-3 min-h-0">
          {filtered.length === 0 ? (
            <div className="flex items-center justify-center py-12 border border-border/50">
              <span className="font-mono text-[13px] text-muted-foreground">
                No providers match your search
              </span>
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
              {visibleProviders.map(cp => (
                <button
                  key={cp.slug}
                  type="button"
                  onClick={() =>
                    handleSelect(
                      {
                        name: cp.name,
                        slug: cp.slug,
                        provider_type: cp.provider_type,
                        base_url: cp.base_url,
                      },
                      cp.api_keys_url,
                    )
                  }
                  className="panel p-3.5 hover:border-brand/30 transition-colors cursor-pointer text-left group flex flex-col gap-1.5"
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="font-mono text-[14px] font-semibold text-foreground group-hover:text-brand transition-colors leading-snug">
                      {cp.name}
                    </div>
                    <div className="flex items-center gap-1.5 shrink-0">
                      {cp.provider_type !== 'openai' && (
                        <span className="font-mono text-[9px] uppercase tracking-wider px-1.5 py-0.5 border border-border/50 text-muted-foreground">
                          {cp.provider_type}
                        </span>
                      )}
                      <a
                        href={cp.api_keys_url}
                        target="_blank"
                        rel="noopener noreferrer"
                        onClick={e => e.stopPropagation()}
                        title="Get API key"
                        className="text-muted-foreground/40 hover:text-brand transition-colors"
                      >
                        <ExternalLinkIcon className="size-3" />
                      </a>
                    </div>
                  </div>
                  <p className="font-mono text-[12px] text-muted-foreground leading-relaxed">
                    {cp.description}
                  </p>
                  <div className="font-mono text-[10px] text-muted-foreground/70 pt-0.5 leading-relaxed">
                    {cp.models}
                  </div>
                </button>
              ))}
            </div>
          )}

          {/* Show more / less */}
          {filtered.length > 8 && (
            <button
              onClick={() => setExpanded(!expanded)}
              className="mt-3 w-full flex items-center justify-center gap-1.5 py-2 border border-border/50 hover:border-brand/20 hover:bg-brand/5 transition-colors cursor-pointer group"
            >
              <span className="font-mono text-[12px] text-muted-foreground group-hover:text-brand transition-colors uppercase tracking-wider">
                {expanded ? 'Show fewer' : `Show all ${filtered.length} providers`}
              </span>
              <ChevronDownIcon
                className={`size-3.5 text-muted-foreground group-hover:text-brand transition-all duration-200 ${expanded ? 'rotate-180' : ''}`}
              />
            </button>
          )}
        </div>

        {/* Footer — custom add */}
        <div className="shrink-0 mt-3 pt-3 border-t border-border">
          <Button
            type="button"
            variant="outline"
            onClick={() => { onCustomAdd(); onOpenChange(false) }}
            className="w-full font-mono text-[12px] uppercase tracking-wider border-border text-muted-foreground hover:text-foreground hover:border-brand/30 transition-colors"
          >
            <PlusIcon className="size-3.5" />
            Add custom provider
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
