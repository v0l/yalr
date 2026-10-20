import { NavLink } from 'react-router-dom'
import { cn } from '@/lib/utils'

const tabs = [
  { to: '/account', label: 'Balance', end: true },
  { to: '/account/models', label: 'Models', end: false },
  { to: '/account/chat', label: 'Chat', end: false },
]

export default function AccountNav() {
  return (
    <nav className="flex gap-0 border-b border-border/50">
      {tabs.map(t => (
        <NavLink
          key={t.to}
          to={t.to}
          end={t.end}
          className={({ isActive }) =>
            cn(
              '-mb-px border-b-[1.5px] px-4 py-2.5 font-mono text-[12px] uppercase tracking-wider transition-colors',
              isActive ? 'border-brand text-brand' : 'border-transparent text-muted-foreground hover:text-foreground/80',
            )
          }
        >
          {t.label}
        </NavLink>
      ))}
    </nav>
  )
}
