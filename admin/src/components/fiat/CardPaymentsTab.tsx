import { useCallback, useEffect, useState } from 'react'
import { fiatApi, type FiatConfig, type FiatOrder, type SavedCard } from '@/api/fiat'
import { currentRole } from '@/lib/role'
import CardTopup from './CardTopup'
import FiatOrdersTable from './FiatOrdersTable'
import SavedCardsPanel from './SavedCardsPanel'

export default function CardPaymentsTab({ config, onPaid }: { config: FiatConfig; onPaid: () => void }) {
  const { isAdmin } = currentRole()
  const [orders, setOrders] = useState<FiatOrder[]>([])
  const [adminOrders, setAdminOrders] = useState<FiatOrder[]>([])
  const [cards, setCards] = useState<SavedCard[]>([])

  const load = useCallback(() => {
    fiatApi.history().then(setOrders).catch(() => undefined)
    fiatApi.cards().then(setCards).catch(() => undefined)
    if (isAdmin) fiatApi.adminOrders().then(setAdminOrders).catch(() => undefined)
  }, [isAdmin])

  useEffect(() => { load() }, [load])

  return (
    <div className="space-y-4">
      <CardTopup config={config} hasSavedCard={cards.length > 0} onPaid={() => { load(); onPaid() }} />
      <SavedCardsPanel config={config} cards={cards} onChanged={load} />
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Your card payments</div>
      <FiatOrdersTable orders={orders} />
      {isAdmin && (
        <>
          <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">All card payments</div>
          <FiatOrdersTable orders={adminOrders} showUser />
        </>
      )}
    </div>
  )
}
