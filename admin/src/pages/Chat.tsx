import { useEffect, useState, useMemo } from 'react'
import { ThreadPrimitive, ComposerPrimitive, MessagePrimitive, ActionBarPrimitive, AssistantRuntimeProvider, useLocalRuntime, AuiIf, type ChatModelAdapter, type ChatModelRunResult, type ThreadAssistantMessagePart } from '@assistant-ui/react'
import { api } from '../api/client'
import type { Model } from '../types'
import { ArrowUpIcon } from 'lucide-react'

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
      <ThreadPrimitive.Root className="flex h-full flex-col items-stretch bg-[#0a0a0b] px-4 font-mono text-foreground">
        <ThreadPrimitive.Viewport className="flex grow flex-col gap-6 overflow-y-scroll pt-12">
          <AuiIf condition={s => s.thread.isEmpty}>
            <div className="flex grow flex-col items-center justify-center">
              <div className="flex h-10 w-10 items-center justify-center border border-[#2a2a2e] bg-[#111113]">
                <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none">
                  <path d="M6 7l4 5-4 5" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
                  <path d="M12 17l4-10" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
                </svg>
              </div>
              <p className="mt-4 text-[#716d66] text-[13px]">How can I help you today?</p>
            </div>
          </AuiIf>

          <ThreadPrimitive.Messages>
            {({ message }) => {
              if (message.role === 'user') return <UserMessage />
              return <AssistantMessage />
            }}
          </ThreadPrimitive.Messages>

          <ThreadPrimitive.ViewportFooter className="sticky bottom-0 mt-auto flex flex-col gap-3 bg-[#0a0a0b] pb-2">
            <ComposerPrimitive.Root className="mx-auto flex w-full max-w-3xl items-end border border-[#2a2a2e] bg-[#111113]">
              <ComposerPrimitive.Input
                placeholder="Message YALR..."
                className="h-10 max-h-40 grow resize-none bg-transparent p-3 text-[13px] text-[#d4d0c8] outline-none placeholder:text-[#454545] font-mono"
              />
              <ComposerPrimitive.Send className="m-1.5 flex size-8 items-center justify-center bg-[#4ce04c]/10 border border-[#4ce04c]/30 text-[#4ce04c] transition-opacity disabled:opacity-20 hover:bg-[#4ce04c]/20">
                <ArrowUpIcon className="size-4" />
              </ComposerPrimitive.Send>
            </ComposerPrimitive.Root>
            <p className="text-center text-[#454545] text-[10px] font-mono uppercase tracking-wider">
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
        <div className="bg-[#1c1c1e] border border-[#2a2a2e] px-4 py-2.5 max-w-[80%] text-right">
          <div className="text-[#d4d0c8] whitespace-pre-wrap text-[13px]">
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
          <div className="flex h-7 w-7 items-center justify-center border border-[#2a2a2e] bg-[#111113] flex-shrink-0 mt-0.5">
            <svg viewBox="0 0 24 24" className="w-3.5 h-3.5" fill="none">
              <path d="M6 7l4 5-4 5" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
              <path d="M12 17l4-10" stroke="#4ce04c" strokeWidth="2.5" strokeLinecap="square"/>
            </svg>
          </div>
          <div className="flex-1 group">
            <div className="text-[#d4d0c8] whitespace-pre-wrap text-[13px] leading-relaxed">
              <MessagePrimitive.Content />
            </div>
            <div className="flex gap-2 mt-1.5 opacity-0 group-hover:opacity-100 transition-opacity">
              <ActionBarPrimitive.Copy asChild>
                <button className="p-1 hover:bg-[#1c1c1e] transition-colors" title="Copy">
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#716d66" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>
                </button>
              </ActionBarPrimitive.Copy>
              <ActionBarPrimitive.Reload asChild>
                <button className="p-1 hover:bg-[#1c1c1e] transition-colors" title="Regenerate">
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#716d66" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>
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
        <p className="font-mono text-[13px] text-[#716d66]">LOADING MODELS...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-6">
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground mb-2 leading-none">CHAT</h1>
        <div className="border border-[#ff3333]/30 bg-[#ff3333]/5 text-[#ff3333] font-mono p-3 text-[13px]">{error}</div>
      </div>
    )
  }

  if (!adapter) {
    return (
      <div className="p-6">
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground mb-2 leading-none">CHAT</h1>
        <p className="font-mono text-[13px] text-[#716d66]">NO MODELS AVAILABLE</p>
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col bg-[#0a0a0b]">
      <div className="px-6 pt-6 pb-4">
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground mb-3 leading-none">CHAT</h1>
        <div className="flex items-center gap-3">
          <label className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66] shrink-0">Model:</label>
          <select
            value={selectedModel}
            onChange={e => setSelectedModel(e.target.value)}
            className="font-mono text-[13px] bg-[#111113] border border-[#2a2a2e] text-[#d4d0c8] px-3 py-1.5 outline-none focus:border-[#4ce04c]/50 transition-colors flex-1 max-w-sm"
            disabled={models.length === 0}
          >
            {models.map(model => (
              <option key={model.id} value={model.id} className="bg-[#111113] text-[#d4d0c8]">{model.id}</option>
            ))}
          </select>
        </div>
      </div>
      <div className="flex-1 px-6 pb-6">
        <div className="h-full border border-[#2a2a2e] bg-[#0a0a0b]">
          <ChatInterface adapter={adapter} />
        </div>
      </div>
    </div>
  )
}
