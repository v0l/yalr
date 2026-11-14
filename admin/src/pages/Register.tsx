import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { authApi, storeSession, type Session } from '../api/auth'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'

export default function Register() {
  const navigate = useNavigate()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function finish(action: () => Promise<Session>) {
    setBusy(true)
    setError(null)
    try {
      const session = await action()
      storeSession(session)
      navigate('/')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Sign up failed')
    } finally {
      setBusy(false)
    }
  }

  function submit(e: React.FormEvent) {
    e.preventDefault()
    if (password !== confirm) return setError('Passwords do not match')
    finish(() => authApi.register(username.trim(), password))
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-background p-8">
      <div className="w-full max-w-sm space-y-6">
        <div>
          <div className="mb-8 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center border border-border bg-card">
              <svg viewBox="0 0 24 24" className="h-5 w-5" fill="none">
                <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square" />
                <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square" />
              </svg>
            </div>
            <span className="font-display text-2xl tracking-[0.08em] text-foreground">YALR</span>
          </div>
          <h1 className="mb-1 font-display text-3xl tracking-[0.06em] text-foreground">CREATE ACCOUNT</h1>
          <p className="font-mono text-[13px] text-muted-foreground">
            A username and a password. No email. Top up and create API keys on Payments once you are in.
          </p>
        </div>

        {error && (
          <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
            <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
          </Alert>
        )}

        <form onSubmit={submit} className="space-y-4">
          {[
            { id: 'r-username', label: 'Username', value: username, set: setUsername, type: 'text', hint: '3-32 letters, digits, dot, dash or underscore' },
            { id: 'r-password', label: 'Password', value: password, set: setPassword, type: 'password', hint: 'At least 8 characters' },
            { id: 'r-confirm', label: 'Repeat password', value: confirm, set: setConfirm, type: 'password', hint: '' },
          ].map(f => (
            <div key={f.id} className="space-y-1.5">
              <Label htmlFor={f.id} className="font-mono text-[11px] uppercase tracking-[0.12em] text-muted-foreground">{f.label}</Label>
              <Input
                id={f.id}
                type={f.type}
                value={f.value}
                onChange={e => f.set(e.target.value)}
                required
                className="h-11 border-border bg-surface font-mono text-[14px]"
              />
              {f.hint && <p className="font-mono text-[10px] text-muted-foreground/70">{f.hint}</p>}
            </div>
          ))}
          <Button
            type="submit"
            disabled={busy}
            className="h-11 w-full border border-brand/40 bg-brand/10 font-mono text-[14px] font-semibold uppercase tracking-[0.05em] text-brand hover:bg-brand/20"
          >
            Sign up
          </Button>
        </form>

        <Button
          type="button"
          variant="outline"
          disabled={busy}
          onClick={() => finish(() => authApi.nostr())}
          className="h-11 w-full border-border font-mono text-[13px] uppercase tracking-[0.05em]"
        >
          Sign up with Nostr
        </Button>

        <p className="font-mono text-[12px] text-muted-foreground">
          Already registered?{' '}
          <Link to="/login" className="uppercase tracking-[0.08em] text-brand hover:text-brand/80">Log in</Link>
        </p>
      </div>
    </div>
  )
}
