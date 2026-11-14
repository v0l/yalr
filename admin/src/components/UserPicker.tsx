import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type { User } from '../types'

const RESULT_LIMIT = 8
const DEBOUNCE_MS = 250

export interface UserPickerProps {
  id?: string
  value: number | null
  valueLabel?: string | null
  onChange: (userId: number | null, label: string | null) => void
}

function userLabel(u: User): string {
  return u.username ?? u.external_id ?? `user ${u.id}`
}

export function UserPicker({ id, value, valueLabel, onChange }: UserPickerProps) {
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<User[]>([])
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (!open) return
    let stale = false
    const timer = setTimeout(() => {
      api.getUsers({ search: query.trim() || undefined, limit: RESULT_LIMIT })
        .then(users => { if (!stale) setResults(users) })
        .catch(() => { if (!stale) setResults([]) })
    }, DEBOUNCE_MS)
    return () => { stale = true; clearTimeout(timer) }
  }, [query, open])

  function pick(userId: number | null, label: string | null) {
    onChange(userId, label)
    setQuery('')
    setOpen(false)
  }

  return (
    <div className="relative">
      <div className="flex items-center gap-2 mb-1.5 font-mono text-[12px]">
        <span className={value ? 'text-brand' : 'text-muted-foreground'}>
          {value ? (valueLabel ?? `user ${value}`) : 'Platform (no owner)'}
        </span>
        {value && (
          <button type="button" onClick={() => pick(null, null)} className="text-[10px] uppercase tracking-wider text-muted-foreground hover:text-destructive">
            Clear
          </button>
        )}
      </div>
      <input
        id={id}
        value={query}
        onChange={e => { setQuery(e.target.value); setOpen(true) }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 150)}
        placeholder="Search users to assign"
        className="h-9 w-full px-3 font-mono text-[12px] bg-surface border border-border text-foreground placeholder:text-muted-foreground"
      />
      {open && (
        <div className="absolute z-50 left-0 right-0 mt-1 max-h-56 overflow-y-auto bg-card border border-border">
          {results.length === 0 ? (
            <div className="px-3 py-2 font-mono text-[12px] text-muted-foreground">No matching users</div>
          ) : results.map(u => (
            <button
              type="button"
              key={u.id}
              onMouseDown={e => e.preventDefault()}
              onClick={() => pick(u.id, userLabel(u))}
              className="block w-full text-left px-3 py-1.5 font-mono text-[12px] hover:bg-surface"
            >
              {userLabel(u)}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
