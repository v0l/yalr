import { useEffect, useState, useSyncExternalStore } from 'react'
import { XIcon } from 'lucide-react'
import { clearVoiceError, getVoicePhase, subscribeVoicePhase } from '../lib/voice-adapters'

export function useVoicePhase() {
  return useSyncExternalStore(subscribeVoicePhase, getVoicePhase, getVoicePhase)
}

function elapsed(since: number): string {
  const total = Math.floor((Date.now() - since) / 1000)
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

/// Recording, transcribing and playback feedback for the chat composer.
/// Dictation is batch, so without this the mic gives no sign it is live.
export default function VoiceStatus() {
  const phase = useVoicePhase()
  const [, tick] = useState(0)

  useEffect(() => {
    if (phase.kind !== 'recording') return
    const id = setInterval(() => tick(t => t + 1), 500)
    return () => clearInterval(id)
  }, [phase.kind])

  if (phase.kind === 'idle') return null

  const base = 'mx-auto flex w-full max-w-3xl items-center gap-2 px-1 font-mono text-[10px] uppercase tracking-[0.1em]'

  if (phase.kind === 'recording') {
    return (
      <div className={`${base} text-brand`}>
        <span className="inline-block size-2 animate-pulse rounded-full bg-brand" />
        <span>Recording {elapsed(phase.since)}</span>
        <span className="text-muted-foreground normal-case tracking-normal">click stop when finished</span>
      </div>
    )
  }

  if (phase.kind === 'transcribing') {
    return (
      <div className={`${base} text-muted-foreground`}>
        <span className="inline-block size-2 animate-pulse rounded-full bg-muted-foreground" />
        <span>Transcribing...</span>
      </div>
    )
  }

  if (phase.kind === 'speaking') {
    return (
      <div className={`${base} text-muted-foreground`}>
        <span className="inline-block size-2 animate-pulse rounded-full bg-muted-foreground" />
        <span>Speaking...</span>
      </div>
    )
  }

  return (
    <div className={`${base} text-destructive`}>
      <span>Voice error:</span>
      <span className="normal-case tracking-normal">{phase.message}</span>
      <button onClick={clearVoiceError} className="ml-auto p-0.5 hover:bg-secondary" title="Dismiss">
        <XIcon className="size-3" />
      </button>
    </div>
  )
}
