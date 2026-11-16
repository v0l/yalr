export type PayoutMethod = 'lightning' | 'revolut_link' | 'revolut_bank'
export type PayoutStatus = 'requested' | 'held' | 'sent' | 'completed' | 'failed' | 'cancelled'

export interface Payout {
  id: number
  user_id: number
  currency: string
  amount: number
  fee: number
  method: PayoutMethod
  destination: string
  status: PayoutStatus
  external_id: string | null
  failure_reason: string | null
  created_at: string
  updated_at: string
  completed_at: string | null
}

export interface CurrencyWithdrawal {
  currency: string
  withdrawable_milli: number
  min_milli: number
  daily_cap_milli: number
  milli_per_unit: number
  methods: PayoutMethod[]
}

export interface WithdrawalOverview {
  enabled: boolean
  hold_days: number
  currencies: CurrencyWithdrawal[]
  payouts: Payout[]
}
