import type { DictationAdapter, SpeechSynthesisAdapter } from '@assistant-ui/react'
import { api } from '../api/client'
import { preferredRecordingMime, recordingFileName } from './audio'

type Listener = () => void

export type VoicePhase =
  | { kind: 'idle' }
  | { kind: 'recording'; since: number }
  | { kind: 'transcribing' }
  | { kind: 'speaking' }
  | { kind: 'error'; message: string }

let phase: VoicePhase = { kind: 'idle' }
const phaseListeners = new Set<Listener>()

function setPhase(next: VoicePhase) {
  phase = next
  phaseListeners.forEach(l => l())
}

export function getVoicePhase(): VoicePhase {
  return phase
}

export function subscribeVoicePhase(callback: Listener): () => void {
  phaseListeners.add(callback)
  return () => {
    phaseListeners.delete(callback)
  }
}

export function clearVoiceError(): void {
  if (phase.kind === 'error') setPhase({ kind: 'idle' })
}

function errorText(error: unknown, fallback: string): string {
  if (error instanceof DOMException && (error.name === 'NotAllowedError' || error.name === 'SecurityError')) {
    return 'Microphone permission denied'
  }
  if (error instanceof DOMException && error.name === 'NotFoundError') return 'No microphone found'
  if (error instanceof Error) return error.message
  return fallback
}

/// Routes assistant-ui's speak action to `/v1/audio/speech` and plays the
/// returned audio, instead of the browser's local voices.
export class RouterSpeechAdapter implements SpeechSynthesisAdapter {
  private readonly model: string
  private readonly voice: string | undefined

  constructor(model: string, voice?: string) {
    this.model = model
    this.voice = voice
  }

  speak(text: string): SpeechSynthesisAdapter.Utterance {
    const listeners = new Set<Listener>()
    const controller = new AbortController()
    let audio: HTMLAudioElement | null = null
    let objectUrl: string | null = null

    const utterance: SpeechSynthesisAdapter.Utterance = {
      status: { type: 'starting' },
      cancel: () => {
        controller.abort()
        audio?.pause()
        end('cancelled')
      },
      subscribe: (callback: Listener) => {
        listeners.add(callback)
        return () => listeners.delete(callback)
      },
    }

    const notify = () => listeners.forEach(l => l())

    const end = (reason: 'finished' | 'cancelled' | 'error', error?: unknown) => {
      if (utterance.status.type === 'ended') return
      if (objectUrl) URL.revokeObjectURL(objectUrl)
      if (reason === 'error') {
        setPhase({ kind: 'error', message: errorText(error, 'Speech synthesis failed') })
      } else if (phase.kind === 'speaking') {
        setPhase({ kind: 'idle' })
      }
      utterance.status = { type: 'ended', reason, error }
      notify()
    }

    setPhase({ kind: 'speaking' })

    api
      .synthesizeSpeech(this.model, text, { voice: this.voice, signal: controller.signal })
      .then(blob => {
        if (controller.signal.aborted) return
        objectUrl = URL.createObjectURL(blob)
        audio = new Audio(objectUrl)
        audio.onended = () => end('finished')
        audio.onerror = () => end('error', audio?.error)
        utterance.status = { type: 'running' }
        notify()
        return audio.play()
      })
      .catch(error => {
        if (controller.signal.aborted) return
        end('error', error)
      })

    return utterance
  }
}

/// Records from the microphone and transcribes through
/// `/v1/audio/transcriptions` when the user stops.
///
/// Server transcription is batch, not streaming, so there are no interim
/// results: `onSpeech` never fires and the whole transcript arrives once via
/// `onSpeechEnd`.
export class RouterDictationAdapter implements DictationAdapter {
  readonly disableInputDuringDictation = false
  private readonly model: string
  private readonly language: string | undefined

  constructor(model: string, language?: string) {
    this.model = model
    this.language = language
  }

  listen(): DictationAdapter.Session {
    const startListeners = new Set<Listener>()
    const endListeners = new Set<(result: DictationAdapter.Result) => void>()
    const speechListeners = new Set<(result: DictationAdapter.Result) => void>()
    const chunks: Blob[] = []
    let recorder: MediaRecorder | null = null
    let stream: MediaStream | null = null
    let cancelled = false

    const session: DictationAdapter.Session = {
      status: { type: 'starting' },
      stop: async () => {
        setPhase({ kind: 'transcribing' })
        const recorded = await finishRecording()
        if (cancelled || !recorded || recorded.size === 0) {
          session.status = { type: 'ended', reason: cancelled ? 'cancelled' : 'stopped' }
          setPhase(cancelled ? { kind: 'idle' } : { kind: 'error', message: 'Nothing was recorded' })
          return
        }
        try {
          const transcript = await api.transcribeAudio(recorded, this.model, {
            language: this.language,
            fileName: recordingFileName(recorded.type),
          })
          session.status = { type: 'ended', reason: 'stopped' }
          // The composer only takes text from onSpeech. onSpeechEnd just tears
          // the session down and discards its result, so emit here first.
          const result = { transcript, isFinal: true }
          speechListeners.forEach(l => l(result))
          endListeners.forEach(l => l(result))
          setPhase(transcript.trim() ? { kind: 'idle' } : { kind: 'error', message: 'No speech detected' })
        } catch (error) {
          session.status = { type: 'ended', reason: 'error' }
          setPhase({ kind: 'error', message: errorText(error, 'Transcription failed') })
        }
      },
      cancel: () => {
        cancelled = true
        stopTracks()
        session.status = { type: 'ended', reason: 'cancelled' }
        setPhase({ kind: 'idle' })
      },
      onSpeechStart: callback => {
        startListeners.add(callback)
        return () => startListeners.delete(callback)
      },
      onSpeechEnd: callback => {
        endListeners.add(callback)
        return () => endListeners.delete(callback)
      },
      onSpeech: callback => {
        speechListeners.add(callback)
        return () => {
          speechListeners.delete(callback)
        }
      },
    }

    const stopTracks = () => {
      if (recorder && recorder.state !== 'inactive') recorder.stop()
      stream?.getTracks().forEach(track => track.stop())
    }

    const finishRecording = (): Promise<Blob | null> =>
      new Promise(resolve => {
        if (!recorder || recorder.state === 'inactive') {
          stopTracks()
          resolve(chunks.length ? new Blob(chunks, { type: chunks[0].type }) : null)
          return
        }
        recorder.onstop = () => {
          stream?.getTracks().forEach(track => track.stop())
          resolve(chunks.length ? new Blob(chunks, { type: chunks[0].type }) : null)
        }
        recorder.stop()
      })

    navigator.mediaDevices
      .getUserMedia({ audio: true })
      .then(micStream => {
        if (cancelled) {
          micStream.getTracks().forEach(track => track.stop())
          return
        }
        stream = micStream
        const mimeType = preferredRecordingMime()
        recorder = new MediaRecorder(micStream, mimeType ? { mimeType } : undefined)
        recorder.ondataavailable = event => {
          if (event.data.size > 0) chunks.push(event.data)
        }
        recorder.start()
        session.status = { type: 'running' }
        setPhase({ kind: 'recording', since: Date.now() })
        startListeners.forEach(l => l())
      })
      .catch(error => {
        session.status = { type: 'ended', reason: 'error' }
        setPhase({ kind: 'error', message: errorText(error, 'Microphone unavailable') })
      })

    return session
  }
}
