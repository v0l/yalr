import type { Modality } from '../../types'
import { cn } from '@/lib/utils'

const MODALITIES: Modality[] = ['text', 'image', 'audio', 'video']

interface ModalityPickerProps {
  input: Modality[]
  output: Modality[]
  onChange: (input: Modality[], output: Modality[]) => void
}

function toggle(list: Modality[], modality: Modality): Modality[] {
  return list.includes(modality)
    ? list.filter(m => m !== modality)
    : MODALITIES.filter(m => m === modality || list.includes(m))
}

function Row({ label, value, onChange }: { label: string; value: Modality[]; onChange: (v: Modality[]) => void }) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-14 font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">{label}</span>
      <div className="flex flex-wrap gap-1.5">
        {MODALITIES.map(m => {
          const on = value.includes(m)
          return (
            <button
              key={m}
              type="button"
              aria-pressed={on}
              onClick={() => onChange(toggle(value, m))}
              className={cn(
                'h-6 border px-2 font-mono text-[11px] uppercase tracking-wider transition-colors',
                on ? 'border-brand/40 bg-brand/15 text-brand' : 'border-border bg-surface text-muted-foreground hover:text-foreground',
              )}
            >
              {m}
            </button>
          )
        })}
      </div>
      {value.length === 0 && <span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground/60">any</span>}
    </div>
  )
}

export default function ModalityPicker({ input, output, onChange }: ModalityPickerProps) {
  return (
    <div className="flex flex-col gap-2">
      <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Modalities</span>
      <Row label="Input" value={input} onChange={v => onChange(v, output)} />
      <Row label="Output" value={output} onChange={v => onChange(input, v)} />
      <p className="font-mono text-[11px] text-muted-foreground">
        Published in /v1/models. Requests needing an unselected modality are rejected. Leave a row empty to accept anything.
      </p>
    </div>
  )
}
