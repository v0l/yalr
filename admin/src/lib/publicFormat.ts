import type { PublicPriceRange, PublicPrice } from '@/api/publicApi'

export function formatAmount(value: number, currency: string): string {
  if (currency === 'sats') return `${Math.round(value).toLocaleString()} sats`
  const digits = value >= 1 ? 2 : 4
  return `${value.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: digits })} ${currency.toUpperCase()}`
}

function span(min: number, max: number, currency: string): string {
  return min === max ? formatAmount(min, currency) : `${formatAmount(min, currency)} - ${formatAmount(max, currency)}`
}

export function rangeLines(ranges: PublicPriceRange[]): { currency: string; input: string; output: string }[] {
  return ranges.map(r => ({
    currency: r.currency,
    input: span(r.min_input, r.max_input, r.currency),
    output: span(r.min_output, r.max_output, r.currency),
  }))
}

export function priceLine(price: PublicPrice | null): string {
  if (!price) return '-'
  return `${formatAmount(price.input_per_1m, price.currency)} in / ${formatAmount(price.output_per_1m, price.currency)} out`
}

export function formatTokens(value: number | null): string {
  if (!value) return '-'
  return value >= 1000 ? `${Math.round(value / 1000)}K` : String(value)
}

export function formatPercent(value: number | null): string {
  return value == null ? '-' : `${value.toFixed(value === 100 ? 0 : 2)}%`
}

export function formatMs(value: number | null): string {
  return value == null ? '-' : `${Math.round(value).toLocaleString()} ms`
}
