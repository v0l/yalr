import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { routingApi, type RoutingPreference, type RoutingSettings as Settings } from '@/api/routingApi'

const OPTIONS: { value: RoutingPreference; hint: string }[] = [
  { value: 'balanced', hint: 'Spread load across healthy providers. The default.' },
  { value: 'cheapest', hint: 'Lowest price first. Equal prices fall back to balanced.' },
  { value: 'fastest', hint: 'Highest recent output tokens per second first.' },
]

function PreferenceSelect({ value, onChange }: { value: RoutingPreference; onChange: (value: RoutingPreference) => void }) {
  return (
    <select
      value={value}
      onChange={e => onChange(e.target.value as RoutingPreference)}
      className="h-9 border border-border bg-surface px-2 font-mono text-[12px] text-foreground"
    >
      {OPTIONS.map(o => <option key={o.value} value={o.value}>{o.value}</option>)}
    </select>
  )
}

export default function RoutingSettings() {
  const [settings, setSettings] = useState<Settings | null>(null)
  const [model, setModel] = useState('')
  const [message, setMessage] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    routingApi.load().then(setSettings).catch(e => setError(e instanceof Error ? e.message : 'Failed to load'))
  }, [])

  async function save(next: Settings) {
    setError(null)
    setMessage(null)
    try {
      setSettings(await routingApi.save(next))
      setMessage('SAVED')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to save')
    }
  }

  if (!settings) return <div className="p-5 font-mono text-[12px] text-muted-foreground">{error ?? 'LOADING...'}</div>

  function addModel() {
    const name = model.trim()
    if (!name || !settings) return
    setModel('')
    save({ ...settings, models: [...settings.models.filter(m => m.model !== name), { model: name, preference: 'cheapest' }] })
  }

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <div className="panel space-y-3 p-4">
        <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Default routing preference</p>
        <PreferenceSelect value={settings.default} onChange={value => save({ ...settings, default: value })} />
        <p className="font-mono text-[11px] text-muted-foreground">
          {OPTIONS.find(o => o.value === settings.default)?.hint} Applies to full-name models such as vendor/model.
          Admin routing configs such as code keep their own strategy. A request can override this with
          provider.sort or the :floor and :nitro model suffixes.
        </p>
      </div>
      <div className="panel space-y-3 p-4">
        <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Per model overrides</p>
        {settings.models.length > 0 && (
          <table className="w-full font-mono text-[12px]">
            <tbody>
              {settings.models.map(m => (
                <tr key={m.model} className="border-b border-border/50">
                  <td className="py-2 text-foreground">{m.model}</td>
                  <td className="py-2">
                    <PreferenceSelect
                      value={m.preference}
                      onChange={value => save({ ...settings, models: settings.models.map(x => x.model === m.model ? { ...x, preference: value } : x) })}
                    />
                  </td>
                  <td className="py-2 text-right">
                    <Button variant="outline" onClick={() => save({ ...settings, models: settings.models.filter(x => x.model !== m.model) })} className="font-mono text-[11px] uppercase">Remove</Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <div className="flex gap-2">
          <Input value={model} onChange={e => setModel(e.target.value)} placeholder="vendor/model" className="font-mono bg-surface border-border text-foreground" />
          <Button onClick={addModel} className="font-mono text-[12px] uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">Add</Button>
        </div>
      </div>
      {message && <p className="font-mono text-[11px] text-brand">{message}</p>}
      {error && <p className="font-mono text-[11px] text-destructive">{error}</p>}
    </div>
  )
}
