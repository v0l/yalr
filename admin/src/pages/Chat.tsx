import { useEffect, useState, useMemo } from 'react'
import { ThreadPrimitive, ComposerPrimitive, MessagePrimitive, ActionBarPrimitive, AssistantRuntimeProvider, useLocalRuntime, AuiIf, type ChatModelAdapter, type ChatModelRunResult, type ThreadAssistantMessagePart } from '@assistant-ui/react'
import { api } from '../api/client'
import type { Model } from '../types'
import { ArrowUpIcon } from 'lucide-react'
import ModelPicker from '../components/ModelPicker'

const createChatModelAdapter = (modelId: string): ChatModelAdapter => {
  return {
    async *run(options) {
      const { messages, abortSignal } = options
      const response = await fetch(`${import.meta.env.VITE_API_URL || window.location.origin}/v1/chat/completions`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${localStorage.getItem('token')}` },
        body: JSON.stringify({ model: modelId, messages: messages.map(m => ({ role: m.role, content: m.content })), stream: true }),
        signal: abortSignal,
      })
      if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`)
      const reader = response.body?.getReader()
      if (!reader) throw new Error('No reader available')
      let buffer = '', accumulatedContent = ''
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
              const content = parsed.choices?.[0]?.delta?.content
              if (content) {
                accumulatedContent += content
                const parts: ThreadAssistantMessagePart[] = [{ type: 'text', text: accumulatedContent }]
                yield { content: parts } satisfies ChatModelRunResult
              }
            } catch {}
          }
        }
      }
    },
  }
}

function ChatInterface({ adapter }: { adapter: ChatModelAdapter }) {
  const runtime = useLocalRuntime(adapter)
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
              return <AssistantMessage />
            }}
          </ThreadPrimitive.Messages>

          <ThreadPrimitive.ViewportFooter className="sticky bottom-0 mt-auto flex flex-col gap-3 bg-background pb-2">
            <ComposerPrimitive.Root className="mx-auto flex w-full max-w-3xl items-end border border-border bg-card">
              <ComposerPrimitive.Input
                placeholder="Message YALR..."
                className="h-10 max-h-40 grow resize-none bg-transparent p-3 text-[13px] text-foreground outline-none placeholder:text-muted-foreground/60 font-mono"
              />
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

function UserMessage() {
  return (
    <div className="flex justify-end">
      <MessagePrimitive.Root>
        <div className="bg-secondary border border-border px-4 py-2.5 max-w-[80%] text-right">
          <div className="text-foreground whitespace-pre-wrap text-[13px]">
            <MessagePrimitive.Content />
          </div>
        </div>
      </MessagePrimitive.Root>
    </div>
  )
}

function AssistantMessage() {
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
              <MessagePrimitive.Content />
            </div>
            <div className="flex gap-2 mt-1.5 opacity-0 group-hover:opacity-100 transition-opacity">
              <ActionBarPrimitive.Copy asChild>
                <button className="p-1 hover:bg-secondary transition-colors" title="Copy">
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>
                </button>
              </ActionBarPrimitive.Copy>
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

export default function Chat() {
  const [models, setModels] = useState<Model[]>([])
  const [selectedModel, setSelectedModel] = useState<string>('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    async function fetchModels() {
      try {
        const response = await api.getModels()
        setModels(response.data)
        if (response.data.length > 0) setSelectedModel(response.data[0].id)
      } catch (e) { setError(e instanceof Error ? e.message : 'Failed to fetch models') }
      finally { setLoading(false) }
    }
    fetchModels()
  }, [])

  const adapter: ChatModelAdapter | undefined = useMemo(() => selectedModel ? createChatModelAdapter(selectedModel) : undefined, [selectedModel])

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
        <div className="flex items-center gap-3">
          <label className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground shrink-0">Model:</label>
          <ModelPicker
            value={selectedModel}
            models={models.map(m => m.id)}
            onChange={setSelectedModel}
            disabled={models.length === 0}
            className="flex-1 max-w-sm"
          />
        </div>
      </div>
      <div className="flex-1 px-6 pb-6">
        <div className="h-full border border-border bg-background">
          <ChatInterface adapter={adapter} />
        </div>
      </div>
    </div>
  )
}
