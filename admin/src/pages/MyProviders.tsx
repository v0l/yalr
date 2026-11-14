import { useEffect, useState } from 'react'
import { userApi } from '@/api/userApi'
import type { Balances, MyProvider, OwnedPrice } from '@/types/userProviders'
import MyProvidersPanel from '@/components/mine/MyProvidersPanel'
import MyPricesPanel from '@/components/mine/MyPricesPanel'
import { formatMilli } from '@/lib/money'

export default function MyProviders() {
  const [providers, setProviders] = useState<MyProvider[]>([])
  const [prices, setPrices] = useState<OwnedPrice[]>([])
  const [balances, setBalances] = useState<Balances | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  async function load() {
    try {
      const [p, pr, b] = await Promise.all([userApi.listProviders(), userApi.listPrices(), userApi.myBalances()])
      setProviders(p)
      setPrices(pr)
      setBalances(b)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load')
    }
  }

  useEffect(() => { load() }, [])

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <div>
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">MY PROVIDERS</h1>
        <p className="font-mono text-[13px] text-muted-foreground">Connect your own endpoints, set prices, earn from consumers</p>
      </div>
      {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}
      {notice && <div className="panel p-3 font-mono text-[12px] text-brand">{notice}</div>}
      <div className="panel p-4">
        <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-2">Balances</div>
        <div className="flex flex-wrap gap-x-8 gap-y-2 font-mono text-[13px] tabular-nums">
          <span>{balances ? formatMilli(balances.balance_msat, 'sats') : '…'}</span>
          {balances?.currencies.map(c => (
            <span key={c.currency}>{formatMilli(c.balance_milli, c.currency)}</span>
          ))}
        </div>
      </div>
      <MyProvidersPanel providers={providers} onChange={load} onError={setError} />
      <MyPricesPanel providers={providers} prices={prices} onChange={load} onError={setError} onSaved={setNotice} />
    </div>
  )
}
