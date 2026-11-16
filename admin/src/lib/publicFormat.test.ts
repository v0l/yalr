import { expect, test } from 'bun:test'
import { badgesFor, formatTokens, lowestTtftKey, providerShownName } from './publicFormat'

test('formatTokens renders millions as M', () => {
  expect(formatTokens(1000000)).toBe('1M')
  expect(formatTokens(1048576)).toBe('1.05M')
  expect(formatTokens(2000000)).toBe('2M')
  expect(formatTokens(1500000)).toBe('1.5M')
  expect(formatTokens(999999)).toBe('1M')
})

test('formatTokens keeps K below a million', () => {
  expect(formatTokens(262144)).toBe('262K')
  expect(formatTokens(512000)).toBe('512K')
})

test('formatTokens handles small and missing values', () => {
  expect(formatTokens(999)).toBe('999')
  expect(formatTokens(null)).toBe('-')
})

test('badgesFor marks only the provider whose key matches', () => {
  const rows = [{ key: 'ripper' }, { key: 'spark' }]
  const model = { cheapest_provider: 'spark', fastest_provider: 'ripper' }
  expect(badgesFor(rows[0], model)).toEqual({ cheapest: false, fastest: true })
  expect(badgesFor(rows[1], model)).toEqual({ cheapest: true, fastest: false })
  expect(badgesFor(rows[1], { cheapest_provider: null, fastest_provider: null })).toEqual({ cheapest: false, fastest: false })
})

test('the routing pick shows the display name for a provider key and picks the lowest TTFT', () => {
  const rows = [
    { key: 'a1', provider: 'Alpha', ttft_p50_ms: 400 },
    { key: 'b2', provider: 'Beta', ttft_p50_ms: 120 },
    { key: 'c3', provider: 'Gamma', ttft_p50_ms: null },
  ]
  expect(providerShownName(rows, 'b2')).toBe('Beta')
  expect(providerShownName(rows, null)).toBe('-')
  expect(lowestTtftKey(rows)).toBe('b2')
  expect(lowestTtftKey([rows[2]])).toBeNull()
})
