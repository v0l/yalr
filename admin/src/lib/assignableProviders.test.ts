import { expect, test } from 'bun:test'
import { assignableProviders, providerOptionLabel } from './assignableProviders'
import type { ProviderListItem } from '../types'

const p = (id: number, owner: number | null, status: 'approved' | 'pending' | 'suspended' = 'approved'): ProviderListItem => ({
  id, name: `p${id}`, slug: `p${id}`, base_url: 'http://x', owner_user_id: owner, owner_username: owner ? 'Kieran' : null, status,
})
const all = [p(1, null), p(2, 26), p(3, 26, 'pending'), p(4, 27)]
const ids = (l: ProviderListItem[]) => l.map(x => x.id)

test('admin on a system config sees approved user providers only', () => {
  expect(ids(assignableProviders(all, null, true))).toEqual([1, 2, 4])
})

test('non-admin keeps system plus own providers', () => {
  expect(ids(assignableProviders(all, 26, false))).toEqual([1, 2, 3])
  expect(ids(assignableProviders(all, null, false))).toEqual([1])
})

test('admin on a user config keeps system plus that owner', () => {
  expect(ids(assignableProviders(all, 26, true))).toEqual([1, 2, 3])
})

test('label carries the owner name', () => {
  expect(providerOptionLabel(p(2, 26))).toBe('p2 (p2), Kieran')
  expect(providerOptionLabel(p(1, null))).toBe('p1 (p1)')
})
