import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { ListingStatus } from '@/api/market'
import type { Model, ModelPricingCreateRequest } from '@/types'
import AdvertiseField from './AdvertiseField'

export default function PricingDialog({
  open,
  onOpenChange,
  editing,
  form,
  setForm,
  saving,
  onSave,
  availableModels,
  unpricedModels,
  pickModel,
  listing,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing: string | null
  form: ModelPricingCreateRequest
  setForm: (form: ModelPricingCreateRequest) => void
  saving: boolean
  onSave: (e: React.FormEvent) => void
  availableModels: Model[]
  unpricedModels: Model[]
  pickModel: (id: string) => void
  listing: ListingStatus | null
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
          <DialogContent className="sm:max-w-lg border-border bg-card">
            <DialogHeader>
              <DialogTitle className="font-display text-xl tracking-[0.04em]">{editing ? 'EDIT PRICING' : 'ADD PRICING'}</DialogTitle>
              <DialogDescription className="font-mono text-[12px] text-muted-foreground">{editing ? 'Update pricing for this model.' : 'Define custom pricing for a model.'}</DialogDescription>
            </DialogHeader>
            <form onSubmit={onSave} className="flex flex-col gap-4">
              {!editing && (
                <div className="flex flex-col gap-1.5"><Label htmlFor="mp-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Model Name</Label><Select value={form.model_name} onValueChange={pickModel}>
                    <SelectTrigger id="mp-name" className="font-mono bg-surface border-border text-foreground"><SelectValue placeholder="Select a model" /></SelectTrigger>
                    <SelectContent className="bg-card border-border max-h-72">
                      <SelectGroup>
                        {unpricedModels.length === 0
                          ? <div className="px-2 py-1.5 font-mono text-[12px] text-muted-foreground">{availableModels.length === 0 ? 'Loading models...' : 'No unpriced models'}</div>
                          : unpricedModels.map(m => <SelectItem key={m.id} value={m.id} className="font-mono text-foreground">{m.id}</SelectItem>)}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
              )}
              <AdvertiseField value={form.is_advertised} onChange={v => setForm({ ...form, is_advertised: v })} listing={listing} />
              <div className="flex items-center gap-4">
                <div className="flex items-center gap-2"><Checkbox id="mp-free" checked={form.is_free} onCheckedChange={c => setForm({ ...form, is_free: !!c })} /><Label htmlFor="mp-free" className="font-mono text-[12px]">FREE</Label></div>
              </div>
              {!form.is_free && (
                <>
                  <div className="grid grid-cols-2 gap-4">
                    <div className="flex flex-col gap-1.5"><Label htmlFor="mp-input" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">In /1M (sats)</Label><Input id="mp-input" type="number" min={0} value={form.price_per_1m_input_sats ?? ''} onChange={e => setForm({ ...form, price_per_1m_input_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                    <div className="flex flex-col gap-1.5"><Label htmlFor="mp-output" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Out /1M (sats)</Label><Input id="mp-output" type="number" min={0} value={form.price_per_1m_output_sats ?? ''} onChange={e => setForm({ ...form, price_per_1m_output_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                  </div>
                  <div className="grid grid-cols-2 gap-4">
                    <div className="flex flex-col gap-1.5"><Label htmlFor="mp-cache-read" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Cache Read /1M (sats)</Label><Input id="mp-cache-read" type="number" min={0} placeholder="same as input" value={form.price_per_1m_cache_read_sats ?? ''} onChange={e => setForm({ ...form, price_per_1m_cache_read_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                    <div className="flex flex-col gap-1.5"><Label htmlFor="mp-cache-write" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Cache Write /1M (sats)</Label><Input id="mp-cache-write" type="number" min={0} placeholder="same as input" value={form.price_per_1m_cache_write_sats ?? ''} onChange={e => setForm({ ...form, price_per_1m_cache_write_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                  </div>
                  <div className="flex flex-col gap-1.5"><Label htmlFor="mp-per-req" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Per Request (sats)</Label><Input id="mp-per-req" type="number" min={0} value={form.price_per_request_sats ?? ''} onChange={e => setForm({ ...form, price_per_request_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                  <div className="grid grid-cols-2 gap-4">
                    <div className="flex flex-col gap-1.5"><Label htmlFor="mp-ctx" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Context Window</Label><Input id="mp-ctx" type="number" min={1} value={form.context_window ?? ''} onChange={e => setForm({ ...form, context_window: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                    <div className="flex flex-col gap-1.5"><Label htmlFor="mp-max-tok" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Max Output Tokens</Label><Input id="mp-max-tok" type="number" min={1} value={form.max_output_tokens ?? ''} onChange={e => setForm({ ...form, max_output_tokens: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                  </div>
                </>
              )}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
                <Button type="submit" disabled={saving || (!editing && !form.model_name)} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{saving ? '...' : editing ? 'UPDATE' : 'CREATE'}</Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
  )
}
