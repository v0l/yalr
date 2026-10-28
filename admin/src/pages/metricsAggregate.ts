import type { WsProviderMetrics, MetricsResponse } from '../types'
import { has, pct } from './metricsHelpers'

export const MAX_SAMPLES = 500

export interface Samples {
  values: number[]
  p90: number | null
}

export interface ModelStats {
  name: string
  requests: number
  successes: number
  failures: number
  ttft: Samples
  latency: Samples
  outTps: Samples
  lastEvent: number
}

export interface AggProvider extends ModelStats {
  models: Map<string, ModelStats>
  inFlight: number
  maxConcurrency: number | null
}

function samples(initial?: number | null): Samples {
  return { values: initial != null ? [initial] : [], p90: initial ?? null }
}

function push(s: Samples, v: number) {
  s.values.push(v)
  if (s.values.length > MAX_SAMPLES) s.values.splice(0, s.values.length - MAX_SAMPLES)
  s.p90 = null
}

export function p90(s: Samples): number | null {
  if (s.p90 === null && s.values.length > 0) s.p90 = pct(s.values, 0.9)
  return s.p90
}

function modelStats(name: string, at: number): ModelStats {
  return { name, requests: 0, successes: 0, failures: 0, ttft: samples(), latency: samples(), outTps: samples(), lastEvent: at }
}

function provider(name: string, at: number): AggProvider {
  return { ...modelStats(name, at), models: new Map(), inFlight: 0, maxConcurrency: null }
}

function record(s: ModelStats, e: WsProviderMetrics) {
  s.lastEvent = e.timestamp_ms
  const ev = e.event
  if (ev === 'Success') { s.requests++; s.successes++ }
  else if (has(ev, 'Failure')) { s.requests++; s.failures++ }
  else if (has(ev, 'TTFT')) push(s.ttft, ev.TTFT as number)
  else if (has(ev, 'TotalLatency')) push(s.latency, ev.TotalLatency as number)
  else if (has(ev, 'OutputTokensPerSecond')) push(s.outTps, ev.OutputTokensPerSecond as number)
}

export function applyEvent(map: Map<string, AggProvider>, e: WsProviderMetrics) {
  let p = map.get(e.provider)
  if (!p) { p = provider(e.provider, e.timestamp_ms); map.set(e.provider, p) }
  record(p, e)
  if (has(e.event, 'ProviderLoad')) {
    const l = e.event.ProviderLoad as { in_flight: number; max_concurrency: number | null }
    p.inFlight = l.in_flight
    p.maxConcurrency = l.max_concurrency
  }
  if (e.model) {
    let m = p.models.get(e.model)
    if (!m) { m = modelStats(e.model, e.timestamp_ms); p.models.set(e.model, m) }
    record(m, e)
  }
}

export function seed(d: MetricsResponse): Map<string, AggProvider> {
  const map = new Map<string, AggProvider>()
  for (const pr of d.providers) {
    if (!pr.provider) continue
    const p = provider(pr.provider, Date.now())
    p.ttft = samples(pr.p90_ttft_ms)
    p.latency = samples(pr.avg_latency_ms)
    p.outTps = samples(pr.p90_tokens_per_second)
    p.inFlight = pr.in_flight ?? 0
    p.maxConcurrency = pr.max_concurrency
    map.set(pr.provider, p)
  }
  return map
}

export function overallP90Ttft(providers: AggProvider[]): number | null {
  const all: number[] = []
  for (const p of providers) for (const v of p.ttft.values) all.push(v)
  return pct(all, 0.9)
}
