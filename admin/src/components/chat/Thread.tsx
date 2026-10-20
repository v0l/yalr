import { useMemo } from 'react'
import { ThreadPrimitive, ComposerPrimitive, MessagePrimitive, MessagePartPrimitive, ErrorPrimitive, ActionBarPrimitive, AssistantRuntimeProvider, useLocalRuntime, AuiIf, type ChatModelAdapter, type ChatModelRunResult, type ThreadAssistantMessagePart, type DictationAdapter, type SpeechSynthesisAdapter } from '@assistant-ui/react'
import { ArrowUpIcon, Loader2Icon, MicIcon, PaperclipIcon, SquareIcon, Volume2Icon } from 'lucide-react'
import VoiceStatus, { useVoicePhase } from '../VoiceStatus'
import PendingReply from './PendingReply'
import { ComposerAttachment, FilePart, ImagePart, MessageAttachment } from './MediaParts'
import { chatAttachments, collectMedia, outputFields, pcm16ToWavDataUrl, toWireMessages, type StreamedMedia } from '../../lib/chat-media'

const mediaComponents = { Text: () => <MessagePartPrimitive.Text />, Image: ImagePart, File: FilePart }

async function errorMessage(response: Response) {
  const body = await response.text()
  try { return JSON.parse(body).error?.message ?? body } catch { return body || `HTTP ${response.status}` }
}

function replyError(message: string | undefined, outputModalities?: readonly string[]) {
  if (message?.includes('empty completion') && outputModalities?.includes('image')) {
    return 'The model finished without returning an image. The provider may have filtered it, so try rewording the prompt.'
  }
  return message ?? 'Request failed'
}

function assistantParts(text: string, media: StreamedMedia, done: boolean): ThreadAssistantMessagePart[] {
  const parts: ThreadAssistantMessagePart[] = []
  const shown = text || media.transcript
  if (shown) parts.push({ type: 'text', text: shown })
  for (const image of media.images) parts.push({ type: 'image', image })
  if (done && media.audio.length > 0) parts.push({ type: 'file', data: pcm16ToWavDataUrl(media.audio), mimeType: 'audio/wav', filename: 'reply.wav' })
  return parts
}

export const createChatModelAdapter = (
  modelId: string,
  outputModalities: readonly string[] | undefined,
  getToken: () => string | null,
  onDone?: () => void,
): ChatModelAdapter => {
  return {
    async *run(options) {
      const { messages, abortSignal } = options
      const response = await fetch(`${import.meta.env.VITE_API_URL || window.location.origin}/v1/chat/completions`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${getToken()}` },
        body: JSON.stringify({ model: modelId, messages: toWireMessages(messages), stream: true, ...outputFields(outputModalities) }),
        signal: abortSignal,
      })
      if (!response.ok) throw new Error(await errorMessage(response))
      const reader = response.body?.getReader()
      if (!reader) throw new Error('No reader available')
      let buffer = '', accumulatedContent = ''
      const media: StreamedMedia = { images: [], audio: [], transcript: '' }
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        const chunk = new TextDecoder().decode(value)
        buffer += chunk
        const lines = buffer.split('\n')
        buffer = lines.pop() || ''
        for (const line of lines) {
          if (line.startsWith('data: ')) {
            const data = line.slice(6)
            if (data === '[DONE]') break
            try {
              const parsed = JSON.parse(data)
              if (parsed.error) throw new Error(replyError(parsed.error.message, outputModalities))
              const delta = parsed.choices?.[0]?.delta ?? {}
              if (delta.content) accumulatedContent += delta.content
              collectMedia(delta, media)
              if (delta.content || delta.images || delta.audio) {
                yield { content: assistantParts(accumulatedContent, media, false) } satisfies ChatModelRunResult
              }
            } catch (e) {
              if (e instanceof Error && !(e instanceof SyntaxError)) throw e
            }
          }
        }
      }
      if (media.audio.length > 0) yield { content: assistantParts(accumulatedContent, media, true) } satisfies ChatModelRunResult
      onDone?.()
    },
  }
}

export function ChatInterface({
  adapter,
  speech,
  dictation,
  pendingLabel,
}: {
  adapter: ChatModelAdapter
  speech?: SpeechSynthesisAdapter
  dictation?: DictationAdapter
  pendingLabel: string
}) {
  const attachments = useMemo(() => chatAttachments(), [])
  const runtime = useLocalRuntime(adapter, { adapters: { speech, dictation, attachments } })
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ThreadPrimitive.Root className="flex h-full flex-col items-stretch bg-background px-4 font-mono text-foreground">
        <ThreadPrimitive.Viewport className="flex grow flex-col gap-6 overflow-y-scroll pt-12">
          <AuiIf condition={s => s.thread.isEmpty}>
            <div className="flex grow flex-col items-center justify-center">
              <div className="flex h-10 w-10 items-center justify-center border border-border bg-card">
                <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none">
                  <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
                  <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
                </svg>
              </div>
              <p className="mt-4 text-muted-foreground text-[13px]">How can I help you today?</p>
            </div>
          </AuiIf>

          <ThreadPrimitive.Messages>
            {({ message }) => {
              if (message.role === 'user') return <UserMessage />
              return <AssistantMessage pendingLabel={pendingLabel} />
            }}
          </ThreadPrimitive.Messages>

          <ThreadPrimitive.ViewportFooter className="sticky bottom-0 mt-auto flex flex-col gap-2 bg-background pb-2">
            <VoiceStatus />
            <ComposerPrimitive.Root className="mx-auto flex w-full max-w-3xl flex-wrap items-end border border-border bg-card">
              <div className="flex w-full flex-wrap gap-1.5 px-1.5 pt-1.5 empty:hidden">
                <ComposerPrimitive.Attachments>{() => <ComposerAttachment />}</ComposerPrimitive.Attachments>
              </div>
              <ComposerPrimitive.AddAttachment
                className="m-1.5 flex size-8 items-center justify-center border border-border text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                title="Attach image, audio, PDF or text"
              >
                <PaperclipIcon className="size-4" />
              </ComposerPrimitive.AddAttachment>
              <ComposerPrimitive.Input
                placeholder="Message YALR..."
                className="h-10 max-h-40 grow resize-none bg-transparent p-3 text-[13px] text-foreground outline-none placeholder:text-muted-foreground/60 font-mono"
              />
              {dictation && <DictationButton />}
              <ComposerPrimitive.Send className="m-1.5 flex size-8 items-center justify-center bg-brand/10 border border-brand/30 text-brand transition-opacity disabled:opacity-20 hover:bg-brand/20">
                <ArrowUpIcon className="size-4" />
              </ComposerPrimitive.Send>
            </ComposerPrimitive.Root>
            <p className="text-center text-muted-foreground/60 text-[10px] font-mono uppercase tracking-wider">
              YALR can make mistakes. Check important info.
            </p>
          </ThreadPrimitive.ViewportFooter>
        </ThreadPrimitive.Viewport>
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>
  )
}

/// One button, not two: mic to start, stop square while recording, and a
/// spinner-free disabled state while the recording is being transcribed.
function DictationButton() {
  const phase = useVoicePhase()

  if (phase.kind === 'recording') {
    return (
      <ComposerPrimitive.StopDictation
        className="m-1.5 flex size-8 items-center justify-center border border-brand/30 bg-brand/10 text-brand transition-colors hover:bg-brand/20"
        title="Stop recording and transcribe"
      >
        <SquareIcon className="size-3.5" />
      </ComposerPrimitive.StopDictation>
    )
  }

  if (phase.kind === 'transcribing') {
    return (
      <span className="m-1.5 flex size-8 items-center justify-center border border-border text-muted-foreground" title="Transcribing">
        <Loader2Icon className="size-4 animate-spin" />
      </span>
    )
  }

  return (
    <ComposerPrimitive.Dictate
      className="m-1.5 flex size-8 items-center justify-center border border-border text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
      title="Record and transcribe"
    >
      <MicIcon className="size-4" />
    </ComposerPrimitive.Dictate>
  )
}

function UserMessage() {
  return (
    <div className="flex justify-end">
      <MessagePrimitive.Root className="flex max-w-[80%] flex-col items-end gap-1.5">
        <div className="flex flex-wrap justify-end gap-1.5 empty:hidden">
          <MessagePrimitive.Attachments>{() => <MessageAttachment />}</MessagePrimitive.Attachments>
        </div>
        <div className="bg-secondary border border-border px-4 py-2.5">
          <div className="text-foreground whitespace-pre-wrap text-[13px]">
            <MessagePrimitive.Content components={mediaComponents} />
          </div>
        </div>
      </MessagePrimitive.Root>
    </div>
  )
}

function AssistantMessage({ pendingLabel }: { pendingLabel: string }) {
  return (
    <div className="flex justify-start">
      <MessagePrimitive.Root>
        <div className="flex gap-3 max-w-3xl">
          <div className="flex h-7 w-7 items-center justify-center border border-border bg-card flex-shrink-0 mt-0.5">
            <svg viewBox="0 0 24 24" className="w-3.5 h-3.5" fill="none">
              <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
              <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
            </svg>
          </div>
          <div className="flex-1 group">
            <div className="text-foreground whitespace-pre-wrap text-[13px] leading-relaxed">
              <MessagePrimitive.Content components={mediaComponents} />
            </div>
            <PendingReply label={pendingLabel} />
            <MessagePrimitive.Error>
              <ErrorPrimitive.Root className="mt-1 border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">
                <ErrorPrimitive.Message />
              </ErrorPrimitive.Root>
            </MessagePrimitive.Error>
            <div className="flex gap-2 mt-1.5 opacity-0 group-hover:opacity-100 transition-opacity">
              <ActionBarPrimitive.Copy asChild>
                <button className="p-1 hover:bg-secondary transition-colors" title="Copy">
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>
                </button>
              </ActionBarPrimitive.Copy>
              <ActionBarPrimitive.Speak asChild>
                <button className="p-1 hover:bg-secondary transition-colors" title="Read aloud">
                  <Volume2Icon className="size-3.5" />
                </button>
              </ActionBarPrimitive.Speak>
              <ActionBarPrimitive.StopSpeaking asChild>
                <button className="p-1 text-brand hover:bg-secondary transition-colors" title="Stop">
                  <SquareIcon className="size-3.5" />
                </button>
              </ActionBarPrimitive.StopSpeaking>
              <ActionBarPrimitive.Reload asChild>
                <button className="p-1 hover:bg-secondary transition-colors" title="Regenerate">
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>
                </button>
              </ActionBarPrimitive.Reload>
            </div>
          </div>
        </div>
      </MessagePrimitive.Root>
    </div>
  )
}

