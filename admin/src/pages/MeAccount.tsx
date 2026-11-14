import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { userApi } from '@/api/userApi'
import { formatMilli } from '@/lib/money'
import type { Balances } from '@/types/userProviders'
import MyApiKeys from './me/MyApiKeys'
import MyTopup from './me/MyTopup'

export default function MeAccount() {
  const user = JSON.parse(localStorage.getItem('user') || '{}')
  const [balances, setBalances] = useState<Balances | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function loadBalances() {
    try { setBalances(await userApi.myBalances()) } catch { setBalances(null) }
  }
  useEffect(() => { loadBalances() }, [])

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <div>
        <h1 className="mb-1 font-display text-[28px] leading-none tracking-[0.04em] text-foreground">ACCOUNT</h1>
        <p className="font-mono text-[13px] text-muted-foreground">Signed in as {user.username ?? 'user'}. Create a key, add a balance, call the API.</p>
      </div>
      {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}
      <div className="panel p-4">
        <div className="mb-2 font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Balance</div>
        <div className="flex flex-wrap gap-x-8 gap-y-2 font-mono text-[18px] tabular-nums text-foreground">
          <span>{balances ? formatMilli(balances.balance_msat, 'sats') : '...'}</span>
          {balances?.currencies.filter(c => c.balance_milli !== 0).map(c => <span key={c.currency}>{formatMilli(c.balance_milli, c.currency)}</span>)}
        </div>
      </div>
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        <MyApiKeys onError={setError} />
        <MyTopup onError={setError} onPaid={loadBalances} />
      </div>
      <div className="panel flex flex-wrap items-center justify-between gap-3 p-4">
        <div>
          <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Become a provider</div>
          <p className="font-mono text-[12px] text-muted-foreground">Connect your own endpoint, set a price and earn when others use it. New providers are checked against a known good provider before they join the shared pool.</p>
        </div>
        <Link to="/my-providers" className="border border-brand/40 bg-brand/10 px-4 py-2 font-mono text-[11px] uppercase tracking-wider text-brand hover:bg-brand/20">My providers</Link>
      </div>
    </div>
  )
}
