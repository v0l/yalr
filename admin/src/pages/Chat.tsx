import { useEffect, useState, useMemo } from 'react'
import { type ChatModelAdapter } from '@assistant-ui/react'
import { api } from '../api/client'
import type { Model } from '../types'
import ModelPicker from '../components/ModelPicker'
import VoiceSettings from '../components/VoiceSettings'
import { loadChatModel, loadVoiceSelection, saveChatModel, saveVoiceSelection, splitAudioModels, type VoiceModelSelection } from '../lib/audio'
import { RouterDictationAdapter, RouterSpeechAdapter } from '../lib/voice-adapters'
import { ChatInterface, createChatModelAdapter } from '../components/chat/Thread'

const adminToken = () => localStorage.getItem('token')

export default function Chat() {
  const [models, setModels] = useState<Model[]>([])
  const [selectedModel, setSelectedModel] = useState<string>('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [voice, setVoice] = useState<VoiceModelSelection>(() => loadVoiceSelection())

  useEffect(() => {
    async function fetchModels() {
      try {
        const response = await api.getModels()
        setModels(response.data)
        if (response.data.length > 0) {
          const remembered = loadChatModel()
          const available = remembered && response.data.some(m => m.id === remembered)
          setSelectedModel(available ? remembered : response.data[0].id)
        }
      } catch (e) { setError(e instanceof Error ? e.message : 'Failed to fetch models') }
      finally { setLoading(false) }
    }
    fetchModels()
  }, [])

  const audioModels = useMemo(() => splitAudioModels(models.map(m => m.id)), [models])

  useEffect(() => {
    const stt = voice.stt && audioModels.stt.includes(voice.stt) ? voice.stt : audioModels.stt[0] ?? null
    const tts = voice.tts && audioModels.tts.includes(voice.tts) ? voice.tts : audioModels.tts[0] ?? null
    if (stt !== voice.stt || tts !== voice.tts) {
      const next = { stt, tts, voice: tts === voice.tts ? voice.voice : '' }
      setVoice(next)
      saveVoiceSelection(next)
    }
  }, [audioModels, voice])

  const updateVoice = (next: VoiceModelSelection) => {
    setVoice(next)
    saveVoiceSelection(next)
  }

  const updateModel = (next: string) => {
    setSelectedModel(next)
    saveChatModel(next)
  }

  const outputModalities = models.find(m => m.id === selectedModel)?.architecture?.output_modalities
  const pendingLabel = outputModalities?.includes('image')
    ? 'Generating image, this can take a few minutes'
    : outputModalities?.includes('audio') ? 'Generating audio' : 'Waiting for the model'
  const adapter: ChatModelAdapter | undefined = useMemo(
    () => selectedModel ? createChatModelAdapter(selectedModel, outputModalities, adminToken) : undefined,
    [selectedModel, outputModalities],
  )
  const speechAdapter = useMemo(
    () => voice.tts ? new RouterSpeechAdapter(voice.tts, voice.voice || undefined) : undefined,
    [voice.tts, voice.voice],
  )
  const dictationAdapter = useMemo(
    () => voice.stt && typeof MediaRecorder !== 'undefined' ? new RouterDictationAdapter(voice.stt) : undefined,
    [voice.stt],
  )

  if (loading) {
    return (
      <div className="p-6">
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground mb-2 leading-none">CHAT</h1>
        <p className="font-mono text-[13px] text-muted-foreground">LOADING MODELS...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-6">
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground mb-2 leading-none">CHAT</h1>
        <div className="border border-destructive/30 bg-destructive/5 text-destructive font-mono p-3 text-[13px]">{error}</div>
      </div>
    )
  }

  if (!adapter) {
    return (
      <div className="p-6">
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground mb-2 leading-none">CHAT</h1>
        <p className="font-mono text-[13px] text-muted-foreground">NO MODELS AVAILABLE</p>
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col bg-background">
      <div className="px-6 pt-6 pb-4">
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground mb-3 leading-none">CHAT</h1>
        <div className="flex flex-wrap items-center gap-3">
          <label className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground shrink-0">Model:</label>
          <ModelPicker
            value={selectedModel}
            models={models.map(m => m.id)}
            onChange={updateModel}
            disabled={models.length === 0}
            className="w-full max-w-sm"
          />
          <VoiceSettings
            sttModels={audioModels.stt}
            ttsModels={audioModels.tts}
            stt={voice.stt}
            tts={voice.tts}
            voice={voice.voice}
            onChange={updateVoice}
          />
        </div>
      </div>
      <div className="flex-1 px-6 pb-6">
        <div className="h-full border border-border bg-background">
          <ChatInterface adapter={adapter} speech={speechAdapter} dictation={dictationAdapter} pendingLabel={pendingLabel} />
        </div>
      </div>
    </div>
  )
}
