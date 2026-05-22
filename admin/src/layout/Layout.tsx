import { Link, Outlet, useLocation, useNavigate } from 'react-router-dom'
import { useTheme } from '../context/ThemeContext'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  LayoutDashboardIcon, SlidersIcon, ActivityIcon,
  UsersIcon, CreditCardIcon, MessageSquareIcon, SunIcon, MoonIcon,
  LogOutIcon, ChevronRightIcon
} from 'lucide-react'

const navigation = [
  { name: 'Dashboard', path: '/', icon: LayoutDashboardIcon },
  { name: 'Config', path: '/config', icon: SlidersIcon },
  { name: 'Metrics', path: '/metrics', icon: ActivityIcon },
  { name: 'Users', path: '/users', icon: UsersIcon },
  { name: 'Payments', path: '/payments', icon: CreditCardIcon },
  { name: 'Chat', path: '/chat', icon: MessageSquareIcon },
]

export default function Layout() {
  const location = useLocation()
  const navigate = useNavigate()
  const { theme, toggleTheme } = useTheme()
  const user = JSON.parse(localStorage.getItem('user') || '{}')

  function handleLogout() {
    localStorage.removeItem('token')
    localStorage.removeItem('user')
    navigate('/login')
  }

  return (
    <div className="min-h-screen flex bg-background">
      {/* ── SIDEBAR ─────────────────────────────────────────── */}
      <aside className="fixed inset-y-0 left-0 z-40 flex w-56 flex-col bg-sidebar border-r border-border/50">
        {/* Logo */}
        <div className="flex items-center gap-2.5 px-5 py-4 border-b border-border/50">
          <div className="flex items-center justify-center w-8 h-8 bg-surface border border-border">
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none">
              <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
              <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
            </svg>
          </div>
          <div>
            <div className="font-display text-lg leading-none tracking-wider text-foreground">YALR</div>
            <div className="text-[9px] uppercase tracking-[0.15em] text-brand/60 font-mono">LLM Router</div>
          </div>
        </div>

        {/* Navigation */}
        <nav className="flex-1 flex flex-col gap-1 px-3 py-4 overflow-y-auto">
          {navigation.map((item) => {
            const isActive = location.pathname === item.path
              || (item.path !== '/' && location.pathname.startsWith(item.path))
            const Icon = item.icon
            return (
              <Link
                key={item.path}
                to={item.path}
                className={cn(
                  'group relative flex items-center gap-3 px-3 py-2 text-[13px] font-mono font-medium transition-colors',
                  'border border-transparent',
                  isActive
                    ? 'bg-brand/10 border-brand/30 text-brand'
                    : 'text-muted-foreground hover:text-foreground/80 hover:bg-surface hover:border-border/50'
                )}
              >
                {/* Active indicator bar */}
                {isActive && (
                  <div className="absolute left-0 top-1/2 -translate-y-1/2 w-0.5 h-5 bg-brand" />
                )}
                <Icon className={cn('size-4 shrink-0', isActive ? 'text-brand' : 'text-muted-foreground/60 group-hover:text-muted-foreground')} />
                <span className="tracking-wide">{item.name}</span>

                {/* Active pointer */}
                {isActive && (
                  <ChevronRightIcon className="ml-auto size-3 text-brand/60" />
                )}
              </Link>
            )
          })}
        </nav>

        {/* Bottom section */}
        <div className="border-t border-border/50 p-3 space-y-2">
          {/* Theme toggle */}
          <button
            onClick={toggleTheme}
            className="flex items-center gap-3 w-full px-3 py-2 text-[13px] font-mono text-muted-foreground hover:text-foreground/80 hover:bg-surface border border-transparent hover:border-border/50 transition-colors"
          >
            {theme === 'light' ? <MoonIcon className="size-4 text-muted-foreground/60" /> : <SunIcon className="size-4 text-muted-foreground/60" />}
            <span className="tracking-wide">{theme === 'light' ? 'Dark Mode' : 'Light Mode'}</span>
            <span className="ml-auto text-[10px] text-brand/70 font-mono tabular-nums">
              {theme === 'dark' ? 'ON' : 'OFF'}
            </span>
          </button>

          {/* User info */}
          <div className="flex items-center gap-3 px-3 py-2">
            <div className="flex items-center justify-center w-7 h-7 bg-surface border border-border text-[11px] font-mono font-bold text-brand">
              {(user.username || 'U')[0].toUpperCase()}
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-[12px] font-mono font-medium text-foreground truncate">
                {user.username || 'User'}
              </div>
              <div className="text-[10px] text-muted-foreground font-mono uppercase tracking-wider">
                {user.isAdmin ? 'ADMIN' : 'USER'}
              </div>
            </div>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={handleLogout}
              className="text-muted-foreground hover:text-destructive hover:bg-transparent"
              title="Logout"
            >
              <LogOutIcon className="size-3.5" />
            </Button>
          </div>
        </div>
      </aside>

      {/* ── MAIN CONTENT ────────────────────────────────────── */}
      <main className="flex-1 ml-56 min-h-screen bg-background">
        <Outlet />
      </main>
    </div>
  )
}
