export type ProviderStatus = 'pending' | 'approved' | 'suspended'

export interface VerificationScores {
  probes: number
  median_prefix: number
  prompt_tokens_compared: number
  prompt_tokens_mismatched: number
  tasks_total: number
  tasks_failed: number
  mean_logprob_gap: number | null
  mean_top5_overlap: number | null
  min_prefix: number | null
  divergence_probes: number
  divergence_hard: number
  mean_divergence: number | null
  candidate_logprobs: boolean
  errors: number
}

export interface VerificationSummary {
  model: string
  outcome: 'passed' | 'failed' | 'needs_review' | 'error' | 'no_reference' | 'reference_error'
  passed: boolean
  reason: string | null
  scores: VerificationScores | null
  reference_provider_id: number | null
  checked_at: string
}

export interface ProbeResult {
  id: string
  fixed: boolean
  prompt: string
  output: string
  reference_output: string
  prefix_score: number
  logprob_gap: number | null
  divergence: number | null
  prompt_tokens: number | null
  reference_prompt_tokens: number | null
  task_passed: boolean | null
  error: string | null
}

export interface VerificationDetail extends VerificationSummary {
  run_id: string
  results: ProbeResult[]
}

export interface ApprovalView {
  slug: string
  status: ProviderStatus
  status_reason: string | null
  status_locked: boolean
  is_reference: boolean
  verifications: VerificationSummary[]
}

export interface MyApiKey {
  id: number
  name: string
  last_four: string
  created_at: string
  expires_at: string | null
  is_active: boolean
}

export interface CreatedApiKey {
  id: number
  name: string
  key: string
  last_four: string
}
