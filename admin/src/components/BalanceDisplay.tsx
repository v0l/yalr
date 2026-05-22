import type { ProviderHealthEntry } from '../types'
import { formatBalance } from '../lib/utils'

export interface BalanceDisplayProps {
  health?: ProviderHealthEntry
  className?: string
}

export function BalanceDisplay({ health, className }: BalanceDisplayProps) {
  if (!health?.balance) return null
  return <span className={`font-mono text-[13px] tabular-nums ${className ?? ''}`}>{formatBalance(health.balance.amount, health.balance.currency)}</span>
}
