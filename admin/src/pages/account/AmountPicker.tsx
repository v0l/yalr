import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

const PRESETS = [1000, 5000, 10000, 50000]
const MIN = 100
const MAX = 1_000_000

export default function AmountPicker({
  label,
  busy,
  onSubmit,
}: {
  label: string
  busy: boolean
  onSubmit: (sats: number) => void
}) {
  const [amount, setAmount] = useState('5000')
  const value = Number.parseInt(amount, 10)
  const valid = Number.isInteger(value) && value >= MIN && value <= MAX

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-4 gap-1.5">
        {PRESETS.map(p => (
          <button
            key={p}
            onClick={() => setAmount(String(p))}
            className={cn(
              'border py-2 font-mono text-[12px] tabular-nums transition-colors',
              value === p ? 'border-brand/40 bg-brand/10 text-brand' : 'border-border bg-surface text-muted-foreground hover:border-brand/20',
            )}
          >
            {p.toLocaleString()}
          </button>
        ))}
      </div>
      <div className="flex gap-2">
        <Input
          type="number"
          min={MIN}
          max={MAX}
          step={1}
          value={amount}
          onChange={e => setAmount(e.target.value)}
          onKeyDown={e => { if (e.key === 'Enter' && valid && !busy) onSubmit(value) }}
          className="h-11 border-border bg-surface font-mono text-[16px] tabular-nums"
        />
        <Button
          disabled={!valid || busy}
          onClick={() => onSubmit(value)}
          className="h-11 border border-brand/40 bg-brand/10 px-5 font-mono text-[12px] uppercase tracking-wider text-brand hover:bg-brand/20 disabled:opacity-40"
        >
          {busy ? 'Creating' : label}
        </Button>
      </div>
      <p className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground/70">
        {MIN.toLocaleString()} to {MAX.toLocaleString()} sats
      </p>
    </div>
  )
}
