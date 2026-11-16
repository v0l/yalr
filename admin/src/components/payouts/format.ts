import { formatMilli } from '@/lib/money'
import type { PayoutStatus } from '@/types/payouts'

export function formatPayoutAmount(amount: number, currency: string): string {
  return formatMilli(amount, currency)
}

export function toMilli(units: string, milliPerUnit: number): number {
  const value = Number(units)
  if (!Number.isFinite(value) || value <= 0) return 0
  return Math.round(value * milliPerUnit)
}

export const statusClass: Record<PayoutStatus, string> = {
  requested: 'border-warning/30 bg-warning/15 text-warning',
  held: 'border-border text-muted-foreground',
  sent: 'border-brand/30 bg-brand/15 text-brand',
  completed: 'border-brand/30 bg-brand/15 text-brand',
  failed: 'border-destructive/30 bg-destructive/15 text-destructive',
  cancelled: 'border-border text-muted-foreground',
}

export function isLink(destination: string): boolean {
  return destination.startsWith('https://')
}
