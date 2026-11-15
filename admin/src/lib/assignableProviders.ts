import type { ProviderListItem } from '../types'

export function assignableProviders(providers: ProviderListItem[], configOwner: number | null | undefined, isAdmin: boolean): ProviderListItem[] {
  const owner = configOwner ?? null
  return providers.filter(p => {
    const providerOwner = p.owner_user_id ?? null
    if (providerOwner === null) return true
    if (isAdmin && owner === null) return (p.status ?? 'approved') === 'approved'
    return providerOwner === owner
  })
}

export function providerOptionLabel(p: ProviderListItem): string {
  const owner = p.owner_user_id != null ? `, ${p.owner_username ?? `user ${p.owner_user_id}`}` : ''
  return `${p.name} (${p.slug})${owner}`
}
