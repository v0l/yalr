import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { useTheme } from '../context/ThemeContext'
import { SunIcon, MoonIcon } from 'lucide-react'

export default function PublicLayout({ children }: { children: React.ReactNode }) {
  const { theme, toggleTheme } = useTheme()
  const signedIn = !!localStorage.getItem('token')

  return (
    <div className="min-h-screen bg-background">
      <header className="flex items-center justify-between border-b border-border/50 bg-sidebar px-4 py-3 sm:px-5">
        <Link to="/models" className="flex items-center gap-2.5">
          <div className="flex h-8 w-8 items-center justify-center border border-border bg-surface">
            <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none">
              <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square" />
              <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square" />
            </svg>
          </div>
          <span className="font-display text-lg leading-none tracking-wider text-foreground">YALR</span>
          <span className="font-mono text-[10px] uppercase tracking-[0.15em] text-brand/70">Models</span>
        </Link>
        <nav className="flex items-center gap-2 font-mono text-[12px] uppercase tracking-wider">
          <button onClick={toggleTheme} className="p-2 text-muted-foreground hover:text-foreground" title="Toggle theme">
            {theme === 'light' ? <MoonIcon className="size-4" /> : <SunIcon className="size-4" />}
          </button>
          {signedIn ? (
            <Button asChild size="sm" variant="outline" className="border-border font-mono text-[11px] uppercase"><Link to="/">Dashboard</Link></Button>
          ) : (
            <>
              <Button asChild size="sm" variant="outline" className="border-border font-mono text-[11px] uppercase"><Link to="/login">Log in</Link></Button>
              <Button asChild size="sm" className="border border-brand/40 bg-brand/10 font-mono text-[11px] uppercase text-brand hover:bg-brand/20"><Link to="/register">Sign up</Link></Button>
            </>
          )}
        </nav>
      </header>
      <main>{children}</main>
    </div>
  )
}
