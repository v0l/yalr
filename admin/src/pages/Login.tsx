import { useState } from 'react'
import { useNavigate, useLocation } from 'react-router-dom'
import { api } from '../api/client'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { cn } from '@/lib/utils'

export default function Login() {
  const navigate = useNavigate()
  const location = useLocation()
  const from = (location.state as { from?: { pathname?: string } } | null)?.from?.pathname || '/'

  const [formData, setFormData] = useState({ username: '', password: '' })
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [focused, setFocused] = useState<string | null>(null)

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setLoading(true)
    setError(null)
    try {
      const response = await api.login(formData)
      localStorage.setItem('token', response.token)
      localStorage.setItem('user', JSON.stringify({ username: response.username, isAdmin: response.isAdmin }))
      navigate(from)
    } catch (e2) {
      setError(e2 instanceof Error ? e2.message : 'Login failed')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="min-h-screen flex bg-[#0a0a0b]">
      {/* Left panel — decorative */}
      <div className="hidden lg:flex flex-1 flex-col justify-between p-12 relative overflow-hidden border-r border-[#1a1a1e]">
        {/* ASCII art / logo */}
        <div>
          <div className="flex items-center gap-3 mb-16">
            <div className="flex items-center justify-center w-10 h-10 bg-[#111113] border border-[#2a2a2e]">
              <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none">
                <path d="M6 7l4 5-4 5" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
                <path d="M12 17l4-10" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
              </svg>
            </div>
            <span className="font-display text-2xl tracking-[0.08em] text-[#d4d0c8]">YALR</span>
          </div>

          <div className="space-y-2 font-mono text-[11px] text-[#454545] tracking-wider">
            <div className="flex items-center gap-2">
              <span className="text-[#4ce04c] animate-pulse-status">▸</span>
              <span>LLM ROUTING ENGINE</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-[#716d66]">▸</span>
              <span>MULTI-PROVIDER LOAD BALANCING</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-[#716d66]">▸</span>
              <span>REAL-TIME METRICS &amp; HEALTH CHECKS</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-[#716d66]">▸</span>
              <span>BITCOIN LIGHTNING PAYMENTS</span>
            </div>
          </div>
        </div>

        {/* Bottom quote */}
        <div className="space-y-1">
          <div className="font-mono text-[10px] text-[#716d66] tracking-[0.12em] uppercase">
            Infrastructure for the sovereign web
          </div>
          <div className="font-mono text-[10px] text-[#454545]">
            v2.0.0
          </div>
        </div>

        {/* Grid overlay */}
        <div className="absolute inset-0 opacity-[0.03] pointer-events-none" style={{
          backgroundImage: 'linear-gradient(#4ce04c 1px, transparent 1px), linear-gradient(90deg, #4ce04c 1px, transparent 1px)',
          backgroundSize: '48px 48px',
        }} />
      </div>

      {/* Right panel — login form */}
      <div className="flex-1 flex items-center justify-center p-8">
        <div className="w-full max-w-sm">
          {/* Mobile logo */}
          <div className="lg:hidden flex items-center gap-3 mb-12">
            <div className="flex items-center justify-center w-10 h-10 bg-[#111113] border border-[#2a2a2e]">
              <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none">
                <path d="M6 7l4 5-4 5" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
                <path d="M12 17l4-10" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
              </svg>
            </div>
            <span className="font-display text-2xl tracking-[0.08em] text-[#d4d0c8]">YALR</span>
          </div>

          <div className="mb-8">
            <h1 className="font-display text-3xl tracking-[0.06em] text-[#d4d0c8] mb-1">AUTHENTICATE</h1>
            <p className="font-mono text-[13px] text-[#716d66]">
              Enter credentials to access the routing control panel.
            </p>
          </div>

          {error && (
            <Alert className="mb-6 border-[#ff3333]/30 bg-[#ff3333]/5 text-[#ff3333] font-mono text-[13px]">
              <AlertDescription>
                <span className="text-[#ff3333]">!</span> {error}
              </AlertDescription>
            </Alert>
          )}

          <form onSubmit={handleSubmit} className="space-y-5">
            <div className="space-y-2">
              <Label
                htmlFor="l-username"
                className="font-mono text-[11px] uppercase tracking-[0.12em] text-[#716d66]"
              >
                Username
              </Label>
              <div className={cn(
                'border transition-colors bg-[#0d0d0f]',
                focused === 'username' ? 'border-[#4ce04c]' : 'border-[#2a2a2e]'
              )}>
                <Input
                  id="l-username"
                  type="text"
                  value={formData.username}
                  onChange={e => setFormData({ ...formData, username: e.target.value })}
                  onFocus={() => setFocused('username')}
                  onBlur={() => setFocused(null)}
                  className="border-0 bg-transparent font-mono text-[14px] text-[#d4d0c8] placeholder:text-[#454545] h-11 px-3 focus-visible:ring-0"
                  placeholder="admin"
                  required
                  autoFocus
                />
              </div>
            </div>

            <div className="space-y-2">
              <Label
                htmlFor="l-password"
                className="font-mono text-[11px] uppercase tracking-[0.12em] text-[#716d66]"
              >
                Password
              </Label>
              <div className={cn(
                'border transition-colors bg-[#0d0d0f]',
                focused === 'password' ? 'border-[#4ce04c]' : 'border-[#2a2a2e]'
              )}>
                <Input
                  id="l-password"
                  type="password"
                  value={formData.password}
                  onChange={e => setFormData({ ...formData, password: e.target.value })}
                  onFocus={() => setFocused('password')}
                  onBlur={() => setFocused(null)}
                  className="border-0 bg-transparent font-mono text-[14px] text-[#d4d0c8] placeholder:text-[#454545] h-11 px-3 focus-visible:ring-0"
                  placeholder="••••••••"
                  required
                />
              </div>
            </div>

            <Button
              type="submit"
              disabled={loading}
              className={cn(
                'w-full h-11 font-mono text-[14px] font-semibold tracking-[0.05em] uppercase transition-all',
                'border border-[#4ce04c]/40 bg-[#4ce04c]/10 text-[#4ce04c]',
                'hover:bg-[#4ce04c]/20 hover:border-[#4ce04c]/60',
                'disabled:opacity-40 disabled:cursor-not-allowed'
              )}
            >
              {loading ? (
                <span className="flex items-center gap-2">
                  <span className="animate-blink">▌</span>
                  AUTHENTICATING
                </span>
              ) : (
                <span className="flex items-center gap-2">
                  LOG IN
                  <span className="text-[10px] opacity-60">→</span>
                </span>
              )}
            </Button>
          </form>

          <div className="mt-8 pt-6 border-t border-[#1a1a1e]">
            <p className="font-mono text-[10px] text-[#454545] text-center tracking-[0.08em] uppercase">
              Yet Another LLM Router
            </p>
          </div>
        </div>
      </div>
    </div>
  )
}
