import { Badge } from '@/components/ui/badge'

export interface HealthBadgeProps {
  state: string
  className?: string
}

export function HealthBadge({ state, className }: HealthBadgeProps) {
  switch (state) {
    case 'healthy': return <Badge className={`bg-brand/15 text-brand border-brand/30 font-mono text-[10px] tracking-wider uppercase ${className ?? ''}`}>HEALTHY</Badge>
    case 'degraded': return <Badge className={`bg-warning/15 text-warning border-warning/30 font-mono text-[10px] tracking-wider uppercase ${className ?? ''}`}>DEGRADED</Badge>
    case 'unhealthy': return <Badge className={`bg-destructive/15 text-destructive border-destructive/30 font-mono text-[10px] tracking-wider uppercase ${className ?? ''}`}>DOWN</Badge>
    default: return <Badge variant="secondary" className={`font-mono text-[10px] ${className ?? ''}`}>{state.toUpperCase()}</Badge>
  }
}
