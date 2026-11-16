export type PaymentTabId = 'balances' | 'pricing' | 'transactions' | 'failover' | 'invoices' | 'withdraw'

export interface PaymentTabAccess {
  isAdmin: boolean
  ownsProvider: boolean
}

export function paymentTabs({ isAdmin, ownsProvider }: PaymentTabAccess): { id: PaymentTabId; label: string }[] {
  const tabs: { id: PaymentTabId; label: string }[] = [{ id: 'balances', label: isAdmin ? 'Balances' : 'Balance' }]
  if (isAdmin || ownsProvider) tabs.push({ id: 'pricing', label: 'Pricing' })
  tabs.push({ id: 'transactions', label: 'Transactions' })
  if (isAdmin || ownsProvider) tabs.push({ id: 'failover', label: 'Failover' })
  tabs.push({ id: 'invoices', label: 'Invoices' })
  if (isAdmin || ownsProvider) tabs.push({ id: 'withdraw', label: 'Withdraw' })
  return tabs
}

export function paymentsSubtitle(isAdmin: boolean): string {
  return isAdmin
    ? 'Balances, pricing, transactions and invoices for every user'
    : 'Your balance, top ups, transactions and invoices'
}
