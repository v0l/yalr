import { expect, test } from 'bun:test'
import { cardsEnabled, isTestMode, topupView } from './topupView'

test('a gateway payment with a pay page is embedded', () => {
  expect(topupView({ pay_url: 'https://pay.example/pay/gw1', bolt11: null })).toBe('embed')
})

test('an LND invoice without a pay page is shown inline', () => {
  expect(topupView({ pay_url: null, bolt11: 'lnbc1' })).toBe('invoice')
})

test('a row with neither shows nothing to pay', () => {
  expect(topupView({ pay_url: null, bolt11: null })).toBe('none')
})

test('cards follow the config flag and an older server without it keeps them on', () => {
  expect(cardsEnabled({ provider: 'lnd', cards: false })).toBe(false)
  expect(cardsEnabled({ provider: 'gateway', cards: true })).toBe(true)
  expect(cardsEnabled({})).toBe(true)
  expect(cardsEnabled({ provider: 'lnd' })).toBe(false)
  expect(cardsEnabled(null)).toBe(false)
})

test('only prod and mainnet are live money', () => {
  expect(isTestMode('sandbox')).toBe(true)
  expect(isTestMode('regtest')).toBe(true)
  expect(isTestMode('prod')).toBe(false)
  expect(isTestMode('mainnet')).toBe(false)
})
