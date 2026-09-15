import ModelPicker from './ModelPicker'

type Props = {
  sttModels: string[]
  ttsModels: string[]
  stt: string | null
  tts: string | null
  onChange: (next: { stt: string | null; tts: string | null }) => void
}

/// Voice model selection for the chat page. Hidden entirely when no audio
/// models are reachable, so a text-only deployment sees no dead controls.
export default function VoiceSettings({ sttModels, ttsModels, stt, tts, onChange }: Props) {
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
            onChange={value => onChange({ stt: value, tts })}
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
            onChange={value => onChange({ stt, tts: value })}
            className="w-56"
          />
        </div>
      )}
    </div>
  )
}
