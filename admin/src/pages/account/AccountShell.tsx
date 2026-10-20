import type { ReactNode } from 'react'
import { Link } from 'react-router-dom'

export default function AccountShell({
  children,
  onSignOut,
  wide = false,
}: {
  children: ReactNode
  onSignOut?: () => void
  wide?: boolean
}) {
  const width = wide ? 'max-w-6xl' : 'max-w-3xl'
  return (
    <div className="min-h-screen bg-background">
      <header className="border-b border-border/50">
        <div className={`mx-auto flex ${width} items-center justify-between px-4 py-4 sm:px-5`}>
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center border border-border bg-surface">
              <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none">
                <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square" />
                <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square" />
              </svg>
            </div>
            <div>
              <div className="font-display text-lg leading-none tracking-wider text-foreground">YALR</div>
              <div className="font-mono text-[9px] uppercase tracking-[0.15em] text-brand/60">Account</div>
            </div>
          </div>
          <div className="flex items-center gap-4">
            <Link to="/login" className="font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground">
              Admin login
            </Link>
            {onSignOut && (
              <button onClick={onSignOut} className="font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground">
                Sign out
              </button>
            )}
          </div>
        </div>
      </header>
      <main className={`mx-auto ${width} space-y-4 p-4 sm:p-5`}>{children}</main>
    </div>
  )
}
