import { expect, test } from 'bun:test'
import { paymentTabs, paymentsSubtitle } from './paymentTabs'

const ids = (isAdmin: boolean, ownsProvider: boolean) => paymentTabs({ isAdmin, ownsProvider }).map(t => t.id)

test('a plain user sees balance, transactions and invoices only', () => {
  expect(ids(false, false)).toEqual(['balances', 'transactions', 'invoices'])
})

test('a provider owner also sees pricing, failover and withdraw', () => {
  expect(ids(false, true)).toEqual(['balances', 'pricing', 'transactions', 'failover', 'invoices', 'withdraw'])
})

test('admins see every tab and none is called card', () => {
  expect(ids(true, false)).toEqual(['balances', 'pricing', 'transactions', 'failover', 'invoices', 'withdraw'])
  expect(ids(true, false)).not.toContain('card')
})

test('the balance tab is singular for users and plural for admins', () => {
  expect(paymentTabs({ isAdmin: false, ownsProvider: false })[0].label).toBe('Balance')
  expect(paymentTabs({ isAdmin: true, ownsProvider: false })[0].label).toBe('Balances')
})

test('the subtitle does not tell a user to manage balances', () => {
  expect(paymentsSubtitle(false)).not.toContain('Manage balances')
  expect(paymentsSubtitle(false)).toContain('Your balance')
})
