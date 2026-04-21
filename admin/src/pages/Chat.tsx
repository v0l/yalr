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
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${localStorage.getItem('token')}`,
        },
        body: JSON.stringify({
          model: modelId,
          messages: messages.map(m => ({ role: m.role, content: m.content })),
          stream: true,
        }),
        signal: abortSignal,
      })

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`)
      }

      const reader = response.body?.getReader()
      if (!reader) throw new Error('No reader available')

      let buffer = ''
      let accumulatedContent = ''

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
                const parts: ThreadAssistantMessagePart[] = [
                  { type: 'text', text: accumulatedContent }
                ]
                
                yield {
                  content: parts,
                } satisfies ChatModelRunResult
              }
            } catch (e) {
              // Skip invalid JSON
            }
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
      <ThreadPrimitive.Root className="dark flex h-full flex-col items-stretch bg-[#212121] px-4 text-foreground">
        <ThreadPrimitive.Viewport className="flex grow flex-col gap-8 overflow-y-scroll pt-16">
          <AuiIf condition={(s) => s.thread.isEmpty}>
            <div className="flex grow flex-col items-center justify-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-3xl border border-white/15 shadow bg-[#212121]">
                <span className="text-white text-xl font-semibold">Y</span>
              </div>
              <p className="mt-4 text-white text-xl">How can I help you today?</p>
            </div>
          </AuiIf>

          <ThreadPrimitive.Messages>
            {({ message }) => {
              if (message.role === 'user') return <UserMessage />
              return <AssistantMessage />
            }}
          </ThreadPrimitive.Messages>

          <ThreadPrimitive.ViewportFooter className="sticky bottom-0 mt-auto flex flex-col gap-4 bg-[#212121] pb-2">
            <ThreadPrimitive.ScrollToBottom className="..." />

            <ComposerPrimitive.Root className="mx-auto flex w-full max-w-3xl items-end rounded-3xl bg-white/5 pl-2">
              <ComposerPrimitive.Input
                placeholder="Message YALR"
                className="h-12 max-h-40 grow resize-none bg-transparent p-3.5 text-sm text-white outline-none placeholder:text-white/50"
              />
              <ComposerPrimitive.Send className="m-2 flex size-8 items-center justify-center rounded-full bg-white transition-opacity disabled:opacity-10">
                <ArrowUpIcon className="size-5 text-black [&_path]:stroke-1 [&_path]:stroke-black" />
              </ComposerPrimitive.Send>
            </ComposerPrimitive.Root>

            <p className="text-center text-[#cdcdcd] text-xs">
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
          <div className="bg-white/10 rounded-2xl px-4 py-2.5 max-w-[80%] text-right">
            <div className="text-white whitespace-pre-wrap">
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
        <div className="flex gap-4 max-w-3xl">
          <div className="flex h-8 w-8 items-center justify-center rounded-full bg-[#212121] border border-white/15 flex-shrink-0">
            <span className="text-white text-xs font-semibold">Y</span>
          </div>
          <div className="flex-1 group">
            <div className="text-white whitespace-pre-wrap">
              <MessagePrimitive.Content />
            </div>
            <div className="flex gap-2 mt-2 opacity-0 group-hover:opacity-100 transition-opacity">
              <ActionBarPrimitive.Copy asChild>
                <button className="p-1.5 hover:bg-white/10 rounded" title="Copy">
                  <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-white/70"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>
                </button>
              </ActionBarPrimitive.Copy>
              <ActionBarPrimitive.Reload asChild>
                <button className="p-1.5 hover:bg-white/10 rounded" title="Regenerate">
                  <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-white/70"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>
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
        if (response.data.length > 0) {
          setSelectedModel(response.data[0].id)
        }
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Failed to fetch models')
      } finally {
        setLoading(false)
      }
    }
    fetchModels()
  }, [])

  const adapter: ChatModelAdapter | undefined = useMemo(() => {
    return selectedModel ? createChatModelAdapter(selectedModel) : undefined
  }, [selectedModel])

  if (loading) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">Chat</h1>
        <p className="text-text-secondary">Loading models...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">Chat</h1>
        <div className="p-4 bg-red-100 border border-red-400 text-red-700 rounded">
          Error: {error}
        </div>
      </div>
    )
  }

  if (!adapter) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">Chat</h1>
        <p className="text-text-secondary">No models available</p>
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col">
      <div className="px-8 pt-8 pb-4">
        <h1 className="text-2xl font-bold mb-4 text-text-primary">Chat</h1>
        
        <div className="mb-4">
          <label className="block text-text-secondary mb-2">Select Model:</label>
          <select
            value={selectedModel}
            onChange={(e) => setSelectedModel(e.target.value)}
            className="w-full md:w-96 p-2 bg-layer-3 border border-border rounded text-text-primary"
            disabled={models.length === 0}
          >
            {models.map((model) => (
              <option key={model.id} value={model.id}>
                {model.id}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="flex-1 px-8 pb-8">
        <div className="h-full border border-border rounded-lg bg-[#212121]">
          <ChatInterface adapter={adapter} />
        </div>
      </div>
    </div>
  )
}
