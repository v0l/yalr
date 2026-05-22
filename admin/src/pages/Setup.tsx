import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { api } from '../api/client'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { cn } from '@/lib/utils'

export default function Setup() {
  const navigate = useNavigate()
  const [formData, setFormData] = useState({ username: '', password: '', confirmPassword: '' })
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [focused, setFocused] = useState<string | null>(null)

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setLoading(true)
    setError(null)

    if (formData.password !== formData.confirmPassword) {
      setError('Passwords do not match')
      setLoading(false)
      return
    }

    try {
      await api.setupFirstUser(formData.username, formData.password)
      navigate('/login')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Setup failed')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="min-h-screen flex bg-background">
      {/* Left panel — decorative */}
      <div className="hidden lg:flex flex-1 flex-col justify-between p-12 relative overflow-hidden border-r border-border/50">
        <div>
          <div className="flex items-center gap-3 mb-16">
            <div className="flex items-center justify-center w-10 h-10 bg-card border border-border">
              <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none">
                <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
                <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
              </svg>
            </div>
            <span className="font-display text-2xl tracking-[0.08em] text-foreground">YALR</span>
          </div>

          <div className="space-y-2 font-mono text-[11px] text-muted-foreground/60 tracking-wider">
            <div className="flex items-center gap-2">
              <span className="text-brand animate-pulse-status">▸</span>
              <span>INITIAL SETUP</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-muted-foreground">▸</span>
              <span>CREATE ADMIN USER</span>
            </div>
          </div>
        </div>

        <div className="space-y-1">
          <div className="font-mono text-[10px] text-muted-foreground tracking-[0.12em] uppercase">
            Infrastructure for the sovereign web
          </div>
          <div className="font-mono text-[10px] text-muted-foreground/60">v2.0.0</div>
        </div>

        <div className="absolute inset-0 opacity-[0.03] pointer-events-none" style={{
          backgroundImage: 'linear-gradient(var(--brand) 1px, transparent 1px), linear-gradient(90deg, var(--brand) 1px, transparent 1px)',
          backgroundSize: '48px 48px',
        }} />
      </div>

      {/* Right panel — setup form */}
      <div className="flex-1 flex items-center justify-center p-8">
        <div className="w-full max-w-sm">
          <div className="lg:hidden flex items-center gap-3 mb-12">
            <div className="flex items-center justify-center w-10 h-10 bg-card border border-border">
              <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none">
                <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
                <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
              </svg>
            </div>
            <span className="font-display text-2xl tracking-[0.08em] text-foreground">YALR</span>
          </div>

          <div className="mb-8">
            <h1 className="font-display text-3xl tracking-[0.06em] text-foreground mb-1">SETUP</h1>
            <p className="font-mono text-[13px] text-muted-foreground">
              Create the initial admin user to get started.
            </p>
          </div>

          {error && (
            <Alert className="mb-6 border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]">
              <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
            </Alert>
          )}

          <form onSubmit={handleSubmit} className="space-y-5">
            <div className="space-y-2">
              <Label htmlFor="s-username" className="font-mono text-[11px] uppercase tracking-[0.12em] text-muted-foreground">Username</Label>
              <div className={cn('border transition-colors bg-surface', focused === 'username' ? 'border-brand' : 'border-border')}>
                <Input id="s-username" type="text" value={formData.username} onChange={e => setFormData({ ...formData, username: e.target.value })}
                  onFocus={() => setFocused('username')} onBlur={() => setFocused(null)}
                  className="border-0 bg-transparent font-mono text-[14px] text-foreground placeholder:text-muted-foreground/60 h-11 px-3 focus-visible:ring-0"
                  placeholder="admin" required autoFocus />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="s-password" className="font-mono text-[11px] uppercase tracking-[0.12em] text-muted-foreground">Password</Label>
              <div className={cn('border transition-colors bg-surface', focused === 'password' ? 'border-brand' : 'border-border')}>
                <Input id="s-password" type="password" value={formData.password} onChange={e => setFormData({ ...formData, password: e.target.value })}
                  onFocus={() => setFocused('password')} onBlur={() => setFocused(null)}
                  className="border-0 bg-transparent font-mono text-[14px] text-foreground placeholder:text-muted-foreground/60 h-11 px-3 focus-visible:ring-0"
                  placeholder="••••••••" required />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="s-confirm" className="font-mono text-[11px] uppercase tracking-[0.12em] text-muted-foreground">Confirm Password</Label>
              <div className={cn('border transition-colors bg-surface', focused === 'confirm' ? 'border-brand' : 'border-border')}>
                <Input id="s-confirm" type="password" value={formData.confirmPassword} onChange={e => setFormData({ ...formData, confirmPassword: e.target.value })}
                  onFocus={() => setFocused('confirm')} onBlur={() => setFocused(null)}
                  className="border-0 bg-transparent font-mono text-[14px] text-foreground placeholder:text-muted-foreground/60 h-11 px-3 focus-visible:ring-0"
                  placeholder="••••••••" required />
              </div>
            </div>
            <Button type="submit" disabled={loading} className={cn(
              'w-full h-11 font-mono text-[14px] font-semibold tracking-[0.05em] uppercase transition-all',
              'border border-brand/40 bg-brand/10 text-brand',
              'hover:bg-brand/20 hover:border-brand/60',
              'disabled:opacity-40 disabled:cursor-not-allowed'
            )}>
              {loading ? (
                <span className="flex items-center gap-2"><span className="animate-blink">▌</span> CREATING...</span>
              ) : (
                <span className="flex items-center gap-2">CREATE ADMIN USER <span className="text-[10px] opacity-60">→</span></span>
              )}
            </Button>
          </form>

          <div className="mt-8 pt-6 border-t border-border/50">
            <p className="font-mono text-[10px] text-muted-foreground/60 text-center tracking-[0.08em] uppercase">
              Yet Another LLM Router
            </p>
          </div>
        </div>
      </div>
    </div>
  )
}
