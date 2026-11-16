import { useState } from 'react'
import ApiKeysPanel from '@/components/ApiKeysPanel'
import { Alert, AlertDescription } from '@/components/ui/alert'

export default function ApiKeys() {
  const [error, setError] = useState<string | null>(null)
  return (
    <div className="space-y-4 p-4 sm:p-5">
      <div>
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">API KEYS</h1>
        <p className="font-mono text-[13px] text-muted-foreground">Keys you use to call the router</p>
      </div>
      {error && <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]"><AlertDescription>{error}</AlertDescription></Alert>}
      <ApiKeysPanel onError={setError} />
    </div>
  )
}
