import { useCallback, useEffect, useState } from 'react'
import { ZapIcon, CreditCardIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { TopupBalanceDialog } from '@/components/TopupBalanceDialog'
import CardTopup from '@/components/fiat/CardTopup'
import FiatOrdersTable from '@/components/fiat/FiatOrdersTable'
import SavedCardsPanel from '@/components/fiat/SavedCardsPanel'
import { fiatApi, type FiatConfig, type FiatOrder, type SavedCard } from '@/api/fiat'
import { tenantApi } from '@/api/tenantApi'
import { formatMilli } from '@/lib/money'
import type { Balances } from '@/types/providerStats'

export function balanceCurrencies(balances: Balances | null, config: FiatConfig | null): string[] {
  const held = balances?.currencies.map(c => c.currency) ?? []
  return ['sats', ...new Set([...(config?.currencies ?? []), ...held])]
}

export function amountMilli(balances: Balances | null, currency: string): number {
  if (!balances) return 0
  if (currency === 'sats') return balances.balance_msat
  return balances.currencies.find(c => c.currency === currency)?.balance_milli ?? 0
}

const tile = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground'
const topupButton = 'font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20 h-8'

export default function BalancePanel({ config, title, onPaid }: { config: FiatConfig | null; title: string; onPaid: () => void }) {
  const [balances, setBalances] = useState<Balances | null>(null)
  const [cards, setCards] = useState<SavedCard[]>([])
  const [orders, setOrders] = useState<FiatOrder[]>([])
  const [lightning, setLightning] = useState(false)
  const [cardCurrency, setCardCurrency] = useState<string | null>(null)

  const load = useCallback(() => {
    tenantApi.myBalances().then(setBalances).catch(() => undefined)
    if (!config) return
    fiatApi.cards().then(setCards).catch(() => undefined)
    fiatApi.history().then(setOrders).catch(() => undefined)
  }, [config])

  useEffect(() => { load() }, [load])

  const paid = () => { load(); onPaid() }
  const fiatCurrencies = config?.currencies ?? []

  return (
    <div className="space-y-4">
      <div className={tile}>{title}</div>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
        {balanceCurrencies(balances, config).map(currency => {
          const isFiat = fiatCurrencies.includes(currency)
          return (
            <div key={currency} className="panel space-y-3 p-4">
              <div className={tile}>{currency}</div>
              <div className="font-mono text-[22px] tabular-nums text-foreground">{formatMilli(amountMilli(balances, currency), currency)}</div>
              {currency === 'sats' && (
                <Button size="sm" onClick={() => setLightning(true)} className={topupButton}><ZapIcon className="size-3" /> Top up</Button>
              )}
              {isFiat && (
                <Button size="sm" onClick={() => setCardCurrency(currency)} className={topupButton}><CreditCardIcon className="size-3" /> Top up</Button>
              )}
            </div>
          )
        })}
      </div>
      {config && cardCurrency && (
        <div className="panel space-y-3 p-4">
          <div className={tile}>Top up {cardCurrency.toUpperCase()} with card</div>
          <CardTopup key={cardCurrency} config={config} currency={cardCurrency} hasSavedCard={cards.length > 0} onPaid={paid} onCancel={() => setCardCurrency(null)} />
        </div>
      )}
      {config && <SavedCardsPanel config={config} cards={cards} onChanged={load} />}
      {config && (
        <>
          <div className={tile}>Your card payments</div>
          <FiatOrdersTable orders={orders} />
        </>
      )}
      <TopupBalanceDialog open={lightning} onOpenChange={setLightning} onPaid={paid} />
    </div>
  )
}
