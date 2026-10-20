import { useEffect, useMemo, useState } from 'react'
import { type ChatModelAdapter } from '@assistant-ui/react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import ModelPicker from '@/components/ModelPicker'
import { ChatInterface, createChatModelAdapter } from '@/components/chat/Thread'
import { accountApi } from '@/api/account'
import { loadAccountModels } from '@/api/account-models'
import { loadChatModel, saveChatModel } from '@/lib/audio'
import type { Model } from '@/types'
import AccountNav from './AccountNav'
import AccountShell from './AccountShell'
import { sats } from './format'

export default function AccountChat({ accountKey, onSignOut }: { accountKey: string; onSignOut: () => void }) {
  const [models, setModels] = useState<Model[]>([])
  const [selected, setSelected] = useState('')
  const [balance, setBalance] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  const refreshBalance = useMemo(
    () => () => accountApi.load(accountKey).then(a => setBalance(a.balance_msat)).catch(() => undefined),
    [accountKey],
  )

  useEffect(() => {
    loadAccountModels(accountKey)
      .then(list => {
        setModels(list)
        const remembered = loadChatModel()
        setSelected(list.some(m => m.id === remembered) ? remembered! : list[0]?.id ?? '')
      })
      .catch(e => setError(e instanceof Error ? e.message : 'Failed to load models'))
      .finally(() => setLoading(false))
    refreshBalance()
  }, [accountKey, refreshBalance])

  const outputModalities = models.find(m => m.id === selected)?.architecture?.output_modalities
  const adapter: ChatModelAdapter | undefined = useMemo(
    () => selected ? createChatModelAdapter(selected, outputModalities, () => accountKey, refreshBalance) : undefined,
    [selected, outputModalities, accountKey, refreshBalance],
  )

  return (
    <AccountShell onSignOut={onSignOut} wide>
      <AccountNav />
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
          <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
        </Alert>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <label className="shrink-0 font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Model:</label>
        <ModelPicker value={selected} models={models.map(m => m.id)} onChange={m => { setSelected(m); saveChatModel(m) }} disabled={models.length === 0} className="w-full max-w-xs" />
        {balance !== null && (
          <span className="ml-auto font-mono text-[12px] tabular-nums text-muted-foreground">Balance <span className="text-brand">{sats(balance)}</span></span>
        )}
      </div>
      <div className="panel h-[calc(100vh-15rem)] min-h-[420px]">
        {loading ? (
          <p className="p-4 font-mono text-[12px] text-muted-foreground">LOADING MODELS...</p>
        ) : adapter ? (
          <ChatInterface adapter={adapter} pendingLabel="Waiting for the model" />
        ) : (
          <p className="p-4 font-mono text-[12px] text-muted-foreground">NO MODELS AVAILABLE</p>
        )}
      </div>
    </AccountShell>
  )
}
