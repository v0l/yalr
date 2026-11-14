import { useState } from 'react'
import { Badge } from '@/components/ui/badge'
import type { ProviderStatus, VerificationDetail, VerificationSummary } from '@/types/userProviders'

const statusStyle: Record<ProviderStatus, string> = {
  approved: 'bg-brand/15 text-brand border-brand/30',
  pending: 'bg-warning/15 text-warning border-warning/30',
  suspended: 'bg-destructive/15 text-destructive border-destructive/30',
}

export function StatusBadge({ status }: { status: ProviderStatus }) {
  return <Badge className={`h-5 px-1.5 py-0 text-[10px] uppercase ${statusStyle[status]}`}>{status}</Badge>
}

const outcomeLabel: Record<string, string> = {
  passed: 'passed',
  failed: 'failed',
  needs_review: 'needs admin review',
  error: 'could not run',
  no_reference: 'no reference',
  reference_error: 'reference down',
}

export function VerificationLine({ v }: { v: VerificationSummary }) {
  const s = v.scores
  const tone = v.outcome === 'passed' ? 'text-brand' : v.outcome === 'failed' ? 'text-destructive' : v.outcome === 'needs_review' ? 'text-warning' : 'text-muted-foreground'
  return (
    <div className="space-y-0.5 font-mono text-[11px]">
      <div className="flex flex-wrap items-baseline gap-x-3">
        <span className="text-foreground">{v.model}</span>
        <span className={tone}>{outcomeLabel[v.outcome] ?? v.outcome}</span>
        {s && v.outcome !== 'no_reference' && (
          <span className="tabular-nums text-muted-foreground">
            prefix {s.median_prefix.toFixed(0)}/32, tokens {s.prompt_tokens_compared - s.prompt_tokens_mismatched}/{s.prompt_tokens_compared},
            tasks {s.tasks_total - s.tasks_failed}/{s.tasks_total}
            {s.mean_divergence != null && `, divergence ${s.mean_divergence.toFixed(2)} (${s.divergence_hard} hard)`}
            {s.mean_logprob_gap !== null && `, logprob gap ${s.mean_logprob_gap.toFixed(2)}`}
            {s.divergence_probes > 0 && !s.candidate_logprobs && ', no logprobs'}
          </span>
        )}
        <span className="text-muted-foreground/60">{new Date(v.checked_at).toLocaleString()}</span>
      </div>
      {v.reason && <div className="text-muted-foreground">{v.reason}</div>}
    </div>
  )
}

export function ProbeTable({ run }: { run: VerificationDetail }) {
  return (
    <table className="w-full font-mono text-[10px]">
      <thead>
        <tr className="border-b border-border/50 text-left uppercase tracking-[0.08em] text-muted-foreground">
          <th className="px-1 py-1 font-medium">Probe</th>
          <th className="px-1 py-1 font-medium">Prefix</th>
          <th className="px-1 py-1 font-medium">Diverge</th>
          <th className="px-1 py-1 font-medium">Tokens</th>
          <th className="px-1 py-1 font-medium">Task</th>
          <th className="px-1 py-1 font-medium">Output</th>
        </tr>
      </thead>
      <tbody>
        {run.results.map(r => (
          <tr key={r.id} className="border-b border-border/30 align-top">
            <td className="px-1 py-1">{r.id}{r.fixed ? '' : ' (random)'}</td>
            <td className="px-1 py-1 tabular-nums">{r.error ? '-' : r.prefix_score}</td>
            <td className={`px-1 py-1 tabular-nums ${(r.divergence ?? 0) > 4 ? 'text-destructive' : ''}`}>{r.divergence == null ? '-' : r.divergence.toFixed(2)}</td>
            <td className="px-1 py-1 tabular-nums">{r.prompt_tokens ?? '-'} / {r.reference_prompt_tokens ?? '-'}</td>
            <td className={`px-1 py-1 ${r.task_passed === false ? 'text-destructive' : ''}`}>{r.task_passed === null ? '-' : r.task_passed ? 'pass' : 'fail'}</td>
            <td className="max-w-80 truncate px-1 py-1 text-muted-foreground" title={r.error ?? r.output}>{r.error ?? r.output}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

export function VerificationHistory({ load }: { load: () => Promise<VerificationDetail[]> }) {
  const [runs, setRuns] = useState<VerificationDetail[] | null>(null)
  const [open, setOpen] = useState<string | null>(null)
  if (!runs) {
    return <button onClick={() => load().then(setRuns).catch(() => setRuns([]))} className="font-mono text-[10px] uppercase tracking-wider text-brand hover:underline">Show probe results</button>
  }
  if (runs.length === 0) return <span className="font-mono text-[11px] text-muted-foreground">No runs yet</span>
  const latest = runs[0]
  return (
    <div className="space-y-2">
      <ProbeTable run={latest} />
      <div className="flex flex-wrap gap-2 font-mono text-[10px] text-muted-foreground">
        {runs.slice(1, 6).map(r => (
          <button key={r.run_id + r.model} onClick={() => setOpen(open === r.run_id + r.model ? null : r.run_id + r.model)} className="underline">
            {r.model} {r.outcome} {new Date(r.checked_at).toLocaleDateString()}
          </button>
        ))}
      </div>
      {runs.filter(r => open === r.run_id + r.model).map(r => <ProbeTable key={r.run_id + r.model} run={r} />)}
    </div>
  )
}
