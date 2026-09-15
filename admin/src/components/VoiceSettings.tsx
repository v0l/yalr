import { useEffect, useState } from 'react'
import ModelPicker from './ModelPicker'
import { api } from '../api/client'
import type { VoiceModelSelection } from '../lib/audio'

type Props = {
  sttModels: string[]
  ttsModels: string[]
  stt: string | null
  tts: string | null
  voice: string
  onChange: (next: VoiceModelSelection) => void
}

/// Voice model selection for the chat page. Hidden entirely when no audio
/// models are reachable, so a text-only deployment sees no dead controls.
export default function VoiceSettings({ sttModels, ttsModels, stt, tts, voice, onChange }: Props) {
  const [voices, setVoices] = useState<string[]>([])
  const [loadingVoices, setLoadingVoices] = useState(false)

  useEffect(() => {
    let cancelled = false
    if (!tts) {
      setVoices([])
      return
    }
    setLoadingVoices(true)
    api
      .getVoices(tts)
      .then(list => {
        if (cancelled) return
        setVoices(list)
        // Backends reject an unknown voice, so never carry one across models.
        if (list.length > 0 && !list.includes(voice)) {
          onChange({ stt, tts, voice: list[0] })
        }
      })
      .catch(() => {
        if (!cancelled) setVoices([])
      })
      .finally(() => {
        if (!cancelled) setLoadingVoices(false)
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tts])

  if (sttModels.length === 0 && ttsModels.length === 0) return null

  const label = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground shrink-0'
  const field =
    'h-8 border border-border bg-card px-2 font-mono text-[11px] text-foreground outline-none focus:border-brand'

  return (
    <div className="flex flex-wrap items-center gap-3">
      {sttModels.length > 0 && (
        <div className="flex items-center gap-2">
          <label className={label}>Mic:</label>
          <ModelPicker
            value={stt ?? ''}
            models={sttModels}
            onChange={next => onChange({ stt: next, tts, voice })}
            className="w-56"
          />
        </div>
      )}
      {ttsModels.length > 0 && (
        <div className="flex items-center gap-2">
          <label className={label}>Voice:</label>
          <ModelPicker
            value={tts ?? ''}
            models={ttsModels}
            onChange={next => onChange({ stt, tts: next, voice: '' })}
            className="w-56"
          />
          {voices.length > 0 ? (
            <select
              value={voice}
              onChange={e => onChange({ stt, tts, voice: e.target.value })}
              className={`${field} w-36`}
              title={`${voices.length} voices published by this model`}
            >
              {voices.map(name => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          ) : (
            <input
              value={voice}
              onChange={e => onChange({ stt, tts, voice: e.target.value })}
              placeholder={loadingVoices ? 'loading voices' : 'voice name'}
              title="This backend does not publish its voices, so type one from its docs"
              className={`${field} w-36 placeholder:text-muted-foreground/60`}
            />
          )}
        </div>
      )}
    </div>
  )
}
