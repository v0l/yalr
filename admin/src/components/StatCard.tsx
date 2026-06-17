import { cn } from '@/lib/utils'

interface StatCardProps {
  label: string
  value: React.ReactNode
  sub?: string
  subGood?: boolean
  subError?: boolean
  color?: 'green' | 'amber' | 'red' | 'default'
}

export default function StatCard({
  label, value, sub, subGood, subError, color = 'default'
}: StatCardProps) {
  const borderColor = color === 'green' ? 'border-brand/20' : color === 'amber' ? 'border-warning/20' : color === 'red' ? 'border-destructive/20' : 'border-border/50'
  const glowColor = color === 'green' ? 'var(--brand)' : color === 'amber' ? 'var(--warning)' : color === 'red' ? 'var(--destructive)' : 'transparent'
  return (
    <div className={cn('panel px-4 py-3.5', borderColor)} style={glowColor !== 'transparent' ? { boxShadow: `inset 0 0 20px ${glowColor}08` } : undefined}>
      <div className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground font-mono mb-1">{label}</div>
      <div className="font-mono text-[28px] tabular-nums leading-none tracking-tight">{value}</div>
      {sub && (
        <div className={cn('mt-1 font-mono text-[11px]', subError ? 'text-destructive' : subGood ? 'text-brand' : 'text-muted-foreground')}>
          {sub}
        </div>
      )}
    </div>
  )
}
