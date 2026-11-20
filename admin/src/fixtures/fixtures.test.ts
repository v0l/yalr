import { expect, test } from 'bun:test'
import type { AccountInvoice } from '@/api/account'
import type { AutoReload, FiatConfig, SavedCard } from '@/api/fiat'
import type { GatewayPaymentWithUser } from '@/types'
import gatewayPayment from './gateway_payment.json'
import accountInvoice from './account_invoice.json'
import savedCard from './saved_card.json'
import autoReload from './auto_reload.json'
import topupConfig from './topup_config.json'

type Keys<T> = Record<keyof T, true>

const payment: Keys<GatewayPaymentWithUser> = {
  id: true, public_id: true, gateway_payment_id: true, user_id: true, kind: true, purpose: true, currency: true,
  amount_milli: true, status: true, bolt11: true, payment_hash: true, pay_url: true, failure_reason: true,
  created_at: true, updated_at: true, expires_at: true, completed_at: true, username: true,
}
const invoice: Keys<AccountInvoice> = { invoice_id: true, purpose: true, amount_sats: true, status: true, created_at: true, paid_at: true }
const card: Keys<SavedCard> = { id: true, brand: true, last_four: true, exp_month: true, exp_year: true }
const reload: Keys<AutoReload> = {
  currency: true, enabled: true, threshold_milli: true, amount_milli: true, monthly_cap_milli: true,
  payment_method_id: true, consecutive_failures: true, disabled_reason: true,
}
const config: Keys<FiatConfig> = { mode: true, currencies: true, min_amount: true, max_amount: true }

const sorted = (o: object) => Object.keys(o).sort()

test('gateway payment fixture matches the TS type', () => expect(sorted(gatewayPayment)).toEqual(sorted(payment)))
test('account invoice fixture matches the TS type', () => expect(sorted(accountInvoice)).toEqual(sorted(invoice)))
test('saved card fixture matches the TS type', () => expect(sorted(savedCard)).toEqual(sorted(card)))
test('auto reload fixture matches the TS type', () => expect(sorted(autoReload)).toEqual(sorted(reload)))
test('top-up config fixture matches the TS type', () => expect(sorted(topupConfig)).toEqual(sorted(config)))
