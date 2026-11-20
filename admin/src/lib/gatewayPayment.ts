import type { GatewayPayment } from '@/types'

export function paymentMethod(p: Pick<GatewayPayment, 'bolt11' | 'currency'>): string {
  if (p.bolt11 || p.currency === 'sats') return 'Lightning'
  return 'Card'
}

export function paymentKind(p: Pick<GatewayPayment, 'kind' | 'purpose'>): string {
  if (p.kind === 'auto_reload') return 'auto-reload'
  if (p.kind === 'anonymous') return p.purpose === 'create' ? 'purchase' : 'anonymous'
  return 'top-up'
}
