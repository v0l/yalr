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
  if (value < 1000) return String(value)
  const thousands = Math.round(value / 1000)
  if (thousands < 1000) return `${thousands}K`
  return `${parseFloat((value / 1000000).toFixed(2))}M`
}

export function formatPercent(value: number | null): string {
  return value == null ? '-' : `${value.toFixed(value === 100 ? 0 : 2)}%`
}

export function formatMs(value: number | null): string {
  return value == null ? '-' : `${Math.round(value).toLocaleString()} ms`
}

export function badgesFor(
  row: { key: string },
  model: { cheapest_provider: string | null; fastest_provider: string | null },
): { cheapest: boolean; fastest: boolean } {
  return { cheapest: row.key === model.cheapest_provider, fastest: row.key === model.fastest_provider }
}

export function providerShownName(rows: { key: string; provider: string }[], key: string | null): string {
  return rows.find(r => r.key === key)?.provider ?? '-'
}

export function lowestTtftKey(rows: { key: string; ttft_p50_ms: number | null }[]): string | null {
  const measured = rows.filter(r => r.ttft_p50_ms != null)
  if (measured.length === 0) return null
  return measured.reduce((a, b) => (b.ttft_p50_ms as number) < (a.ttft_p50_ms as number) ? b : a).key
}
