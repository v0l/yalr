const STT_PATTERNS = [/whisper/i, /transcribe/i, /\bstt\b/i, /parakeet/i, /distil-whisper/i, /canary/i]
const TTS_PATTERNS = [/\btts\b/i, /kokoro/i, /speech/i, /piper/i, /xtts/i, /orpheus/i, /voice/i]

/// Model ids carry no capability metadata, so audio models are recognised by
/// name. Anything unmatched still works if the user types it in by hand.
function matches(id: string, patterns: RegExp[]): boolean {
  return patterns.some(p => p.test(id))
}

export function isTranscriptionModel(id: string): boolean {
  return matches(id, STT_PATTERNS) && !/\btts\b/i.test(id)
}

export function isSpeechModel(id: string): boolean {
  return matches(id, TTS_PATTERNS) && !/whisper|transcribe/i.test(id)
}

export function splitAudioModels(ids: string[]): { stt: string[]; tts: string[] } {
  return {
    stt: ids.filter(isTranscriptionModel),
    tts: ids.filter(isSpeechModel),
  }
}

/// Pick the MIME type the browser will actually record in. Chrome and Firefox
/// give webm/opus, Safari gives mp4/aac, and whisper backends accept both.
export function preferredRecordingMime(): string | undefined {
  if (typeof MediaRecorder === 'undefined') return undefined
  const candidates = ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4', 'audio/ogg;codecs=opus']
  return candidates.find(type => MediaRecorder.isTypeSupported(type))
}

export function recordingFileName(mime: string | undefined): string {
  if (!mime) return 'recording.webm'
  if (mime.includes('mp4')) return 'recording.mp4'
  if (mime.includes('ogg')) return 'recording.ogg'
  return 'recording.webm'
}

const STORAGE_KEY = 'voiceModels'

export type VoiceModelSelection = { stt: string | null; tts: string | null }

export function loadVoiceSelection(): VoiceModelSelection {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { stt: null, tts: null }
    const parsed = JSON.parse(raw)
    return { stt: parsed?.stt ?? null, tts: parsed?.tts ?? null }
  } catch {
    return { stt: null, tts: null }
  }
}

export function saveVoiceSelection(selection: VoiceModelSelection): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(selection))
}
