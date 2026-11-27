import type { FiatConfig, FiatOrder } from '@/api/fiat'

export type TopupView = 'embed' | 'invoice' | 'none'

export function topupView(order: Pick<FiatOrder, 'pay_url' | 'bolt11'>): TopupView {
  if (order.pay_url) return 'embed'
  if (order.bolt11) return 'invoice'
  return 'none'
}

export function cardsEnabled(config: Pick<FiatConfig, 'cards' | 'provider'> | null): boolean {
  if (!config) return false
  return config.cards ?? config.provider !== 'lnd'
}

export function isTestMode(mode: FiatConfig['mode']): boolean {
  return mode !== 'prod' && mode !== 'mainnet'
}
