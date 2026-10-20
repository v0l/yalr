export function sats(msat: number): string {
  const value = msat / 1000
  const digits = Math.abs(value) < 10 && value % 1 !== 0 ? 3 : 0
  return `${value.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits })} sats`
}

export function when(timestamp: string): string {
  const d = new Date(timestamp.includes('T') ? timestamp : `${timestamp.replace(' ', 'T')}Z`)
  return Number.isNaN(d.getTime()) ? timestamp : d.toLocaleString()
}
