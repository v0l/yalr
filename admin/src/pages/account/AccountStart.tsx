import { useEffect, useState } from 'react'
import { CheckIcon, CopyIcon, KeyRoundIcon } from 'lucide-react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { accountApi, accountKey, AccountAuthError, type CreatedInvoice } from '@/api/account'
import { Skeleton } from '@/components/ui/skeleton'
import { loadPublicModels } from '@/api/account-models'
import type { Model } from '@/types'
import AccountShell from './AccountShell'
import ModelPriceTable from './ModelPriceTable'
import AmountPicker from './AmountPicker'
import PayInvoice from './PayInvoice'

export default function AccountStart({ onKey }: { onKey: (key: string) => void }) {
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [invoice, setInvoice] = useState<CreatedInvoice | null>(null)
  const [newKey, setNewKey] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [existing, setExisting] = useState('')
  const [models, setModels] = useState<Model[] | null>(null)

  useEffect(() => { loadPublicModels().then(setModels).catch(() => setModels([])) }, [])

  async function buy(sats: number) {
    setBusy(true)
    setError(null)
    try {
      setInvoice(await accountApi.createInvoice(sats, 'create'))
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not create invoice')
    } finally {
      setBusy(false)
    }
  }

  async function signIn() {
    const key = existing.trim()
    setBusy(true)
    setError(null)
    try {
      await accountApi.load(key)
      accountKey.set(key)
      onKey(key)
    } catch (e) {
      setError(e instanceof AccountAuthError ? 'That API key was not recognised' : e instanceof Error ? e.message : 'Sign in failed')
    } finally {
      setBusy(false)
    }
  }

  async function copy() {
    if (!newKey) return
    await navigator.clipboard.writeText(newKey)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  if (newKey) {
    return (
      <AccountShell>
        <div className="panel space-y-4 p-4">
          <h1 className="font-display text-[24px] leading-none tracking-[0.04em] text-foreground">PAYMENT RECEIVED</h1>
          <p className="font-mono text-[12px] leading-relaxed text-muted-foreground">
            This API key is your account. There is no email or password, and nobody can recover it for you. Save it now.
          </p>
          <div className="relative">
            <div className="break-all border border-brand/40 bg-surface p-3 pr-12 font-mono text-[12px] leading-relaxed text-brand">{newKey}</div>
            <Button variant="ghost" size="icon" onClick={copy} className="absolute right-1 top-1/2 size-8 -translate-y-1/2" title="Copy key">
              {copied ? <CheckIcon className="size-3.5 text-brand" /> : <CopyIcon className="size-3.5" />}
            </Button>
          </div>
          <Button
            onClick={() => { accountKey.set(newKey); onKey(newKey) }}
            className="h-11 w-full border border-brand/40 bg-brand/10 font-mono text-[12px] uppercase tracking-wider text-brand hover:bg-brand/20"
          >
            I have saved my key, continue
          </Button>
        </div>
      </AccountShell>
    )
  }

  return (
    <AccountShell wide>
      <div>
        <h1 className="mb-1 font-display text-[28px] leading-none tracking-[0.04em] text-foreground">GET AN API KEY</h1>
        <p className="font-mono text-[13px] text-muted-foreground">Pay with Lightning. The amount you pay becomes your balance.</p>
      </div>

      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
          <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
        </Alert>
      )}

      {invoice ? (
        <PayInvoice
          invoice={invoice}
          onCancel={() => setInvoice(null)}
          onPaid={status => status.api_key ? setNewKey(status.api_key) : setError('Paid, but no key was returned. Contact the operator with your invoice id: ' + invoice.invoice_id)}
        />
      ) : (
        <div className="panel p-4"><AmountPicker label="Create invoice" busy={busy} onSubmit={buy} /></div>
      )}

      <div className="panel space-y-3 p-4">
        <div className="flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
          <KeyRoundIcon className="size-3.5" /> Already have a key
        </div>
        <div className="flex gap-2">
          <Input
            type="password"
            value={existing}
            onChange={e => setExisting(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter' && existing.trim() && !busy) signIn() }}
            placeholder="sk-..."
            autoComplete="off"
            className="h-11 border-border bg-surface font-mono text-[13px]"
          />
          <Button
            variant="outline"
            disabled={!existing.trim() || busy}
            onClick={signIn}
            className="h-11 border-border px-5 font-mono text-[12px] uppercase tracking-wider text-foreground"
          >
            Sign in
          </Button>
        </div>
      </div>

      <div className="space-y-2">
        <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Models and pricing (sats)</p>
        <div className="panel overflow-x-auto">
          {!models ? <Skeleton className="m-4 h-24 bg-secondary" /> : models.length === 0 ? (
            <p className="p-6 text-center font-mono text-[12px] text-muted-foreground">No models listed</p>
          ) : <ModelPriceTable models={models} />}
        </div>
      </div>
    </AccountShell>
  )
}
