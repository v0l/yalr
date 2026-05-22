import { PencilIcon, TrashIcon, WalletIcon, KeyIcon } from 'lucide-react'
import type { Provider, ProviderHealthEntry } from '../types'
import { Button } from '@/components/ui/button'
import { HealthBadge } from '@/components/HealthBadge'
import { BalanceDisplay } from '@/components/BalanceDisplay'
import { Badge } from '@/components/ui/badge'

/* ═══════════════════════════════════════════════════════════════ */
/*  ProviderCard                                                   */
/* ═══════════════════════════════════════════════════════════════ */

export interface ProviderCardProps {
  provider: Provider
  onEdit: (p: Provider) => void
  onDelete: (p: Provider) => void
  onTopup: (p: Provider) => void
  onGenerateKey: (p: Provider) => void
}

export function ProviderCard({ provider, onEdit, onDelete, onTopup, onGenerateKey }: ProviderCardProps) {
  const h: ProviderHealthEntry | undefined = provider.health
  const state = h?.health_state ?? 'unknown'
  const accent =
    state === 'healthy' ? 'var(--brand)' :
    state === 'degraded' ? 'var(--warning)' :
    state === 'unhealthy' ? 'var(--destructive)' :
    'var(--muted-foreground)'
  const cardShadow = `${accent}14`

  return (
    <div
      className="panel group relative overflow-hidden transition-all hover:shadow-[0_0_24px_var(--card-shadow)]"
      style={{
        '--card-shadow': cardShadow,
        borderLeft: `3px solid ${accent}`,
      } as React.CSSProperties}
    >
      {/* scan-line overlay on hover */}
      <div
        className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100 transition-opacity duration-500"
        style={{ background: `linear-gradient(180deg, transparent 0%, ${accent}03 50%, transparent 100%)` }}
      />

      <div className="relative p-3">
        {/* ── Header row: dot + name/slug → balance ───── */}
        <div className="flex items-center gap-2 mb-2">
          <span
            className="w-2 h-2 rounded-full shrink-0"
            style={{ backgroundColor: accent, boxShadow: `0 0 6px ${accent}80` }}
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-1.5">
              <span className="font-mono text-[13px] font-bold leading-none truncate">{provider.name}</span>
              <span className="font-mono text-[10px] text-muted-foreground truncate">{provider.slug}</span>
            </div>
          </div>
          <div className="shrink-0 text-right">
            <BalanceDisplay health={h} className="text-[18px] font-bold" />
          </div>
        </div>

        {/* ── Config row: type + health + URL ──────────── */}
        <div className="flex items-center gap-1.5 mb-2 flex-wrap">
          <Badge variant="secondary" className="font-mono text-[10px] tracking-wider bg-secondary text-muted-foreground border-border px-1.5 py-0 h-5">
            {provider.provider_type.toUpperCase()}
          </Badge>
          <HealthBadge state={state} />
        </div>

        <div className="font-mono text-[10px] text-muted-foreground/60 truncate" title={provider.base_url}>
          {provider.base_url.replace(/^https?:\/\//, '').replace(/\/v\d+$/, '')}
        </div>

        {/* ── Actions ────────────────────────────────────── */}
        <div className="flex items-center gap-1 mt-2.5 pt-2.5 border-t border-border/40">
          {(provider.provider_type === 'routstr' || provider.provider_type === 'ppq') && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onTopup(provider)}
              className="h-6 font-mono text-[10px] uppercase tracking-wider border-border/60 text-muted-foreground hover:text-brand hover:border-brand/40 transition-colors gap-1 px-1.5"
            >
              <WalletIcon className="size-3" /> TOP-UP
            </Button>
          )}
          {provider.provider_type === 'ppq' && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onGenerateKey(provider)}
              className="h-6 font-mono text-[10px] uppercase tracking-wider border-border/60 text-muted-foreground hover:text-brand hover:border-brand/40 transition-colors gap-1 px-1.5"
            >
              <KeyIcon className="size-3" /> KEY
            </Button>
          )}
          <div className="flex-1" />
          <Button
            variant="ghost"
            size="icon-xs"
            onClick={() => onEdit(provider)}
            className="text-muted-foreground hover:text-foreground h-6 w-6"
            title="Edit"
          >
            <PencilIcon className="size-3" />
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            onClick={() => onDelete(provider)}
            className="text-muted-foreground hover:text-destructive h-6 w-6"
            title="Delete"
          >
            <TrashIcon className="size-3" />
          </Button>
        </div>
      </div>
    </div>
  )
}
