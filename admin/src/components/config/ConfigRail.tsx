import { PlusIcon } from 'lucide-react'
import type { RoutingConfigFull } from '../../types'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

interface ConfigRailProps {
  configs: RoutingConfigFull[]
  selectedId: number | null
  onSelect: (id: number) => void
  onCreate: () => void
}

export default function ConfigRail({ configs, selectedId, onSelect, onCreate }: ConfigRailProps) {
  return (
    <div className="panel flex flex-col">
      <div className="flex items-center justify-between border-b border-border/60 px-3 py-2.5">
        <span className="font-mono text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
          Configs <span className="text-border">·</span> {configs.length}
        </span>
        <Button
          size="icon-xs"
          variant="ghost"
          onClick={onCreate}
          title="Add config"
          className="text-brand hover:bg-brand/10"
        >
          <PlusIcon className="size-3.5" />
        </Button>
      </div>

      <nav className="flex flex-col">
        {configs.map(config => {
          const active = config.id === selectedId
          const activeCount = config.providers.filter(p => p.is_active).length
          return (
            <button
              key={config.id}
              onClick={() => onSelect(config.id)}
              className={cn(
                'group relative flex flex-col gap-1 border-b border-border/40 px-3 py-2.5 text-left transition-colors',
                active ? 'bg-brand/10' : 'hover:bg-surface',
              )}
            >
              {active && <span className="absolute inset-y-0 left-0 w-0.5 bg-brand" />}
              <span
                className={cn(
                  'truncate font-mono text-[13px] font-medium',
                  active ? 'text-brand' : 'text-foreground',
                )}
              >
                {config.name}
              </span>
              <span className="flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                <span className="truncate">{config.strategy.replace(/_/g, ' ')}</span>
                <span className="text-border">·</span>
                <span className="tabular-nums">{config.providers.length}p</span>
                <span
                  className={cn(
                    'ml-auto size-1.5 shrink-0 rounded-full',
                    !config.health_check_enabled
                      ? 'bg-muted-foreground/40'
                      : activeCount > 0
                        ? 'bg-brand'
                        : 'bg-warning',
                  )}
                  title={
                    !config.health_check_enabled
                      ? 'Health checks off'
                      : `${activeCount}/${config.providers.length} active`
                  }
                />
              </span>
            </button>
          )
        })}
      </nav>
    </div>
  )
}
