import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { API_BASE_URL } from '@/api/client'

interface ListedModel {
  id: string
  status: 'listed' | 'needs_price' | 'needs_full_name'
}

interface ListingReport {
  provider: string
  public: boolean
  models: ListedModel[]
  short_names: string[]
}

const LABEL: Record<ListedModel['status'], string> = {
  listed: 'Listed',
  needs_price: 'Needs a price',
  needs_full_name: 'Needs a full repo id (org/name)',
}

export default function ProviderListingPanel({ slug }: { slug: string }) {
  const [report, setReport] = useState<ListingReport | null>(null)

  useEffect(() => {
    const token = localStorage.getItem('token')
    fetch(`${API_BASE_URL}/api/providers/${encodeURIComponent(slug)}/listing`, { headers: token ? { Authorization: `Bearer ${token}` } : {} })
      .then(r => (r.ok ? r.json() : null))
      .then(setReport)
      .catch(() => setReport(null))
  }, [slug])

  if (!report || report.models.length === 0) return null
  const pending = report.models.filter(m => m.status !== 'listed')

  return (
    <div className="panel space-y-2 p-4">
      <div className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">Marketplace listing</div>
      {report.short_names.length > 0 && (
        <div className="border border-warning/30 bg-warning/10 p-2 font-mono text-[12px] text-warning">
          This provider returns short model names ({report.short_names.slice(0, 3).join(', ')}). Short names are not listed publicly.
          Serve the model under its Hugging Face repo id, for example vLLM <code>--served-model-name org/name</code>.
        </div>
      )}
      {pending.length === 0 && <div className="font-mono text-[12px] text-muted-foreground">All models are listed.</div>}
      {report.models.map(m => (
        <div key={m.id} className="flex items-center justify-between gap-2 font-mono text-[12px]">
          <span className="truncate text-foreground">{m.id}</span>
          {m.status === 'needs_price' ? (
            <Link to={`/payments?tab=pricing&model=${encodeURIComponent(m.id)}&provider=${encodeURIComponent(slug)}`} className="text-warning hover:underline">
              {LABEL[m.status]}: set price
            </Link>
          ) : (
            <span className={m.status === 'listed' ? 'text-brand' : 'text-warning'}>{LABEL[m.status]}</span>
          )}
        </div>
      ))}
    </div>
  )
}
