import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { loadAccountModels } from '@/api/account-models'
import type { Model } from '@/types'
import AccountNav from './AccountNav'
import AccountShell from './AccountShell'
import ModelPriceTable from './ModelPriceTable'

export default function AccountModels({ accountKey, onSignOut }: { accountKey: string; onSignOut: () => void }) {
  const [models, setModels] = useState<Model[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadAccountModels(accountKey).then(setModels).catch(e => setError(e instanceof Error ? e.message : 'Failed to load models'))
  }, [accountKey])

  return (
    <AccountShell onSignOut={onSignOut} wide>
      <AccountNav />
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
          <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
        </Alert>
      )}
      <div className="panel overflow-x-auto">
        {!models ? <Skeleton className="m-4 h-24 bg-secondary" /> : models.length === 0 ? (
          <p className="p-6 text-center font-mono text-[12px] text-muted-foreground">No models are available to this key</p>
        ) : (
          <ModelPriceTable models={models} />
        )}
      </div>
      <div className="panel space-y-2 p-4">
        <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Use the API</p>
        <p className="break-all font-mono text-[12px] text-foreground">Base URL: <span className="text-brand">{window.location.origin}/v1</span></p>
        <p className="font-mono text-[11px] text-muted-foreground">
          OpenAI compatible. Send your key as a bearer token. <Link to="/account/chat" className="text-brand hover:text-brand/80">Try it in chat</Link>.
        </p>
      </div>
    </AccountShell>
  )
}
