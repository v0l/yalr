export function formatMilli(milli: number, currency: string): string {
  if (currency === 'sats') return `${(milli / 1000).toLocaleString()} sats`
  const units = milli / 100_000
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency: currency.toUpperCase(), minimumFractionDigits: 2, maximumFractionDigits: 5 }).format(units)
  } catch {
    return `${units.toFixed(5)} ${currency.toUpperCase()}`
  }
}

export function formatPrice(value: number | null, currency: string): string {
  if (value === null) return '—'
  return currency === 'sats' ? `${value.toLocaleString()} sats` : formatMilli(value, currency)
}
