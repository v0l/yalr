import { Label } from '@/components/ui/label'
import { cn } from '@/lib/utils'

export type ExtraBodyParse = { value: Record<string, unknown>; error: null } | { value: null; error: string }

export function parseExtraBody(text: string): ExtraBodyParse {
  const trimmed = text.trim()
  if (!trimmed) return { value: {}, error: null }
  try {
    const parsed: unknown = JSON.parse(trimmed)
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { value: null, error: 'Must be a JSON object, for example {"provider": {"order": ["slug"]}}' }
    }
    return { value: parsed as Record<string, unknown>, error: null }
  } catch (e) {
    return { value: null, error: `Invalid JSON: ${e instanceof Error ? e.message : String(e)}` }
  }
}

export function formatExtraBody(body: Record<string, unknown> | undefined | null): string {
  return body && Object.keys(body).length > 0 ? JSON.stringify(body, null, 2) : ''
}

interface ExtraBodyFieldProps {
  id: string
  value: string
  onChange: (text: string) => void
  hint?: string
  rows?: number
}

export function ExtraBodyField({ id, value, onChange, hint, rows = 5 }: ExtraBodyFieldProps) {
  const { error } = parseExtraBody(value)
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id} className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
        Extra Body<span className="font-normal text-muted-foreground tracking-normal"> (JSON object merged into every request)</span>
      </Label>
      <textarea
        id={id}
        value={value}
        rows={rows}
        spellCheck={false}
        onChange={e => onChange(e.target.value)}
        placeholder={'{"provider": {"order": ["slug"], "allow_fallbacks": false}}'}
        aria-invalid={error ? true : undefined}
        className={cn(
          'w-full resize-y border bg-surface p-2 font-mono text-[12px] text-foreground outline-none focus:border-brand/60',
          error ? 'border-destructive/60' : 'border-border',
        )}
      />
      {error ? (
        <p className="font-mono text-[10px] text-destructive">{error}</p>
      ) : (
        hint && <p className="font-mono text-[10px] text-muted-foreground">{hint}</p>
      )}
    </div>
  )
}
