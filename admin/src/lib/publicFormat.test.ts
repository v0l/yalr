import { expect, test } from 'bun:test'
import { formatTokens } from './publicFormat'

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
