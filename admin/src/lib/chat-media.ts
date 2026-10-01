import {
  CompositeAttachmentAdapter,
  SimpleImageAttachmentAdapter,
  SimpleTextAttachmentAdapter,
  type AttachmentAdapter,
  type ThreadMessage,
} from '@assistant-ui/react'

type PendingAttachment = Awaited<ReturnType<SimpleImageAttachmentAdapter['add']>>
type CompleteAttachment = Awaited<ReturnType<SimpleImageAttachmentAdapter['send']>>

const readDataUrl = (file: File) =>
  new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(reader.result as string)
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(file)
  })

const pending = (file: File, type: string): PendingAttachment => ({
  id: `${file.name}-${file.size}-${file.lastModified}`,
  type,
  name: file.name,
  contentType: file.type,
  file,
  status: { type: 'requires-action', reason: 'composer-send' },
})

class AudioAttachmentAdapter implements AttachmentAdapter {
  accept = 'audio/wav,audio/x-wav,audio/mpeg,audio/mp3'
  async add({ file }: { file: File }) { return pending(file, 'file') }
  async send(attachment: PendingAttachment): Promise<CompleteAttachment> {
    const url = await readDataUrl(attachment.file)
    const format = /wav/.test(attachment.file.type) ? 'wav' : 'mp3'
    return {
      ...attachment,
      status: { type: 'complete' },
      content: [{ type: 'audio', audio: { data: url.slice(url.indexOf(',') + 1), format } }],
    }
  }
  async remove() {}
}

class PdfAttachmentAdapter implements AttachmentAdapter {
  accept = 'application/pdf'
  async add({ file }: { file: File }) { return pending(file, 'document') }
  async send(attachment: PendingAttachment): Promise<CompleteAttachment> {
    return {
      ...attachment,
      status: { type: 'complete' },
      content: [{ type: 'file', filename: attachment.name, data: await readDataUrl(attachment.file), mimeType: 'application/pdf' }],
    }
  }
  async remove() {}
}

export const chatAttachments = () =>
  new CompositeAttachmentAdapter([
    new SimpleImageAttachmentAdapter(),
    new AudioAttachmentAdapter(),
    new PdfAttachmentAdapter(),
    new SimpleTextAttachmentAdapter(),
  ])

type WirePart =
  | { type: 'text'; text: string }
  | { type: 'image_url'; image_url: { url: string } }
  | { type: 'input_audio'; input_audio: { data: string; format: string } }
  | { type: 'file'; file: { filename?: string; file_data: string } }

type WireMessage = { role: string; content: string | WirePart[] }

function userParts(message: ThreadMessage): WirePart[] {
  const parts = [...message.content, ...(message.attachments ?? []).flatMap(a => a.content ?? [])]
  return parts.flatMap((part): WirePart[] => {
    switch (part.type) {
      case 'text': return [{ type: 'text', text: part.text }]
      case 'image': return [{ type: 'image_url', image_url: { url: part.image } }]
      case 'audio': return [{ type: 'input_audio', input_audio: part.audio }]
      case 'file': return [{ type: 'file', file: { filename: part.filename, file_data: part.data } }]
      default: return []
    }
  })
}

export function toWireMessages(messages: readonly ThreadMessage[]): WireMessage[] {
  return messages.map(message => {
    if (message.role !== 'user') {
      const text = message.content.flatMap(p => (p.type === 'text' ? [p.text] : [])).join('')
      return { role: message.role, content: text }
    }
    const parts = userParts(message)
    const textOnly = parts.every(p => p.type === 'text')
    return {
      role: 'user',
      content: textOnly ? parts.map(p => (p as { text: string }).text).join('\n') : parts,
    }
  })
}

export function outputFields(outputModalities: readonly string[] | undefined) {
  const wanted = (outputModalities ?? []).filter(m => m === 'image' || m === 'audio')
  if (wanted.length === 0) return {}
  return {
    modalities: ['text', ...wanted],
    ...(wanted.includes('audio') ? { audio: { voice: 'alloy', format: 'pcm16' } } : {}),
  }
}

export function pcm16ToWavDataUrl(base64Chunks: readonly string[], sampleRate = 24000): string {
  const bytes = base64Chunks.flatMap(chunk => Array.from(atob(chunk), c => c.charCodeAt(0)))
  const header = new DataView(new ArrayBuffer(44))
  const text = (offset: number, value: string) => [...value].forEach((c, i) => header.setUint8(offset + i, c.charCodeAt(0)))
  text(0, 'RIFF')
  header.setUint32(4, 36 + bytes.length, true)
  text(8, 'WAVE')
  text(12, 'fmt ')
  header.setUint32(16, 16, true)
  header.setUint16(20, 1, true)
  header.setUint16(22, 1, true)
  header.setUint32(24, sampleRate, true)
  header.setUint32(28, sampleRate * 2, true)
  header.setUint16(32, 2, true)
  header.setUint16(34, 16, true)
  text(36, 'data')
  header.setUint32(40, bytes.length, true)
  const wav = new Uint8Array(44 + bytes.length)
  wav.set(new Uint8Array(header.buffer), 0)
  wav.set(bytes, 44)
  let binary = ''
  for (let i = 0; i < wav.length; i += 0x8000) binary += String.fromCharCode(...wav.subarray(i, i + 0x8000))
  return `data:audio/wav;base64,${btoa(binary)}`
}

export type StreamedMedia = { images: string[]; audio: string[]; transcript: string }

export function collectMedia(delta: { images?: { image_url?: { url?: string } }[]; audio?: { data?: string; transcript?: string } }, media: StreamedMedia) {
  for (const image of delta.images ?? []) {
    const url = image.image_url?.url
    if (url && !media.images.includes(url)) media.images.push(url)
  }
  if (delta.audio?.data) media.audio.push(delta.audio.data)
  if (delta.audio?.transcript) media.transcript += delta.audio.transcript
}
