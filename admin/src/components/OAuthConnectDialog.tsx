import { useState, useEffect } from 'react'
import { ExternalLinkIcon, ShieldCheckIcon, Loader2Icon } from 'lucide-react'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { api } from '../api/client'
import type { OAuthProviderKind } from '../types'

interface OAuthKindMeta {
  kind: OAuthProviderKind
  label: string
  sub: string
  defaultName: string
  defaultSlug: string
}

const KINDS: OAuthKindMeta[] = [
  { kind: 'anthropic', label: 'Claude Pro / Max', sub: 'Anthropic subscription via claude.ai', defaultName: 'Claude Max', defaultSlug: 'claude-max' },
  { kind: 'openai', label: 'ChatGPT Plus / Pro', sub: 'OpenAI subscription via Codex', defaultName: 'ChatGPT', defaultSlug: 'chatgpt' },
]

type Step = 'configure' | 'authorize'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** When set, the dialog re-authenticates an existing provider instead of creating one. */
  reauth?: { slug: string; kind: OAuthProviderKind; name: string }
  onConnected: (message: string) => void
}

export function OAuthConnectDialog({ open, onOpenChange, reauth, onConnected }: Props) {
  const [step, setStep] = useState<Step>('configure')
  const [kind, setKind] = useState<OAuthProviderKind>('anthropic')
  const [name, setName] = useState('Claude Max')
  const [slug, setSlug] = useState('claude-max')
  const [authorizeUrl, setAuthorizeUrl] = useState('')
  const [instructions, setInstructions] = useState('')
  const [oauthState, setOauthState] = useState('')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Reset whenever the dialog opens.
  useEffect(() => {
    if (!open) return
    setError(null)
    setCode('')
    setBusy(false)
    if (reauth) {
      setKind(reauth.kind)
      setName(reauth.name)
      setSlug(reauth.slug)
      setStep('configure')
      // Auto-start the authorize step for re-auth.
      void beginReauth(reauth.slug, reauth.kind)
    } else {
      setStep('configure')
      selectKind('anthropic')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  function selectKind(k: OAuthProviderKind) {
    const meta = KINDS.find(m => m.kind === k)!
    setKind(k)
    setName(meta.defaultName)
    setSlug(meta.defaultSlug)
  }

  async function beginReauth(s: string, k: OAuthProviderKind) {
    setBusy(true)
    setError(null)
    try {
      const res = await api.reauthOAuth(s, k)
      setAuthorizeUrl(res.authorize_url)
      setInstructions(res.instructions)
      setOauthState(res.state)
      setStep('authorize')
      window.open(res.authorize_url, '_blank', 'noopener,noreferrer')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to start re-auth')
    } finally {
      setBusy(false)
    }
  }

  async function handleStart() {
    setBusy(true)
    setError(null)
    try {
      const res = await api.startOAuth(kind, name.trim(), slug.trim())
      setAuthorizeUrl(res.authorize_url)
      setInstructions(res.instructions)
      setOauthState(res.state)
      setStep('authorize')
      window.open(res.authorize_url, '_blank', 'noopener,noreferrer')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to start OAuth flow')
    } finally {
      setBusy(false)
    }
  }

  async function handleComplete() {
    if (!code.trim()) {
      setError('Paste the authorization code or redirect URL')
      return
    }
    setBusy(true)
    setError(null)
    try {
      await api.completeOAuth(oauthState, code.trim())
      onConnected(reauth ? 'SUBSCRIPTION RE-AUTHENTICATED' : 'SUBSCRIPTION CONNECTED')
      onOpenChange(false)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to complete OAuth flow')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg border-border bg-card">
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-[0.04em] text-brand flex items-center gap-2">
            <ShieldCheckIcon className="size-5" />
            {reauth ? 'RE-AUTHENTICATE' : 'CONNECT SUBSCRIPTION'}
          </DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            Use a Claude or ChatGPT subscription instead of an API key. Tokens refresh automatically.
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[12px]">
            <AlertDescription className="break-words">{error}</AlertDescription>
          </Alert>
        )}

        {step === 'configure' && !reauth && (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-2">
              {KINDS.map(m => (
                <button
                  key={m.kind}
                  type="button"
                  onClick={() => selectKind(m.kind)}
                  className={
                    'text-left p-3 border font-mono transition-colors ' +
                    (kind === m.kind
                      ? 'border-brand/50 bg-brand/10'
                      : 'border-border bg-surface hover:bg-surface/70')
                  }
                >
                  <div className="text-[13px] font-bold text-foreground">{m.label}</div>
                  <div className="text-[10px] text-muted-foreground mt-1 leading-tight">{m.sub}</div>
                </button>
              ))}
            </div>

            <div className="space-y-2">
              <Label className="text-[10px] uppercase tracking-wider text-muted-foreground font-mono">Name</Label>
              <Input value={name} onChange={e => setName(e.target.value)} className="font-mono text-[13px] bg-surface border-border" />
            </div>
            <div className="space-y-2">
              <Label className="text-[10px] uppercase tracking-wider text-muted-foreground font-mono">Slug</Label>
              <Input value={slug} onChange={e => setSlug(e.target.value.toLowerCase().replace(/\s+/g, '-'))} className="font-mono text-[13px] bg-surface border-border" />
              <p className="text-[10px] text-muted-foreground font-mono">Used for prefixed routing, e.g. <span className="text-brand">{slug || 'slug'}/model</span></p>
            </div>
          </div>
        )}

        {step === 'authorize' && (
          <div className="space-y-4">
            <Alert className="border-brand/20 bg-brand/5 text-muted-foreground font-mono text-[12px]">
              <AlertDescription>{instructions}</AlertDescription>
            </Alert>

            <Button
              type="button"
              onClick={() => window.open(authorizeUrl, '_blank', 'noopener,noreferrer')}
              className="w-full font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20"
            >
              <ExternalLinkIcon className="size-3.5" /> Open Authorization Page
            </Button>

            <div className="space-y-2">
              <Label className="text-[10px] uppercase tracking-wider text-muted-foreground font-mono">Authorization Code / Redirect URL</Label>
              <Input
                value={code}
                onChange={e => setCode(e.target.value)}
                placeholder="paste code, CODE#STATE, or full URL"
                className="font-mono text-[13px] bg-surface border-border"
                autoFocus
              />
            </div>
          </div>
        )}

        <DialogFooter>
          {step === 'configure' && !reauth && (
            <Button onClick={handleStart} disabled={busy || !name.trim() || !slug.trim()} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
              {busy ? <><Loader2Icon className="size-3.5 animate-spin" /> STARTING…</> : 'AUTHORIZE'}
            </Button>
          )}
          {step === 'authorize' && (
            <Button onClick={handleComplete} disabled={busy} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
              {busy ? <><Loader2Icon className="size-3.5 animate-spin" /> CONNECTING…</> : 'CONNECT'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
