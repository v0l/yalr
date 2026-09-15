import ModelPicker from './ModelPicker'
import { defaultVoiceFor, type VoiceModelSelection } from '../lib/audio'

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
  if (sttModels.length === 0 && ttsModels.length === 0) return null

  return (
    <div className="flex flex-wrap items-center gap-3">
      {sttModels.length > 0 && (
        <div className="flex items-center gap-2">
          <label className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground shrink-0">
            Mic:
          </label>
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
          <label className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground shrink-0">
            Voice:
          </label>
          <ModelPicker
            value={tts ?? ''}
            models={ttsModels}
            onChange={next => onChange({ stt, tts: next, voice: defaultVoiceFor(next) || voice })}
            className="w-56"
          />
          <input
            value={voice}
            onChange={e => onChange({ stt, tts, voice: e.target.value })}
            placeholder="voice name"
            title="Most backends require a voice name, e.g. alloy, af_bella, Zephyr"
            className="h-8 w-32 border border-border bg-card px-2 font-mono text-[11px] text-foreground outline-none placeholder:text-muted-foreground/60 focus:border-brand"
          />
        </div>
      )}
    </div>
  )
}
