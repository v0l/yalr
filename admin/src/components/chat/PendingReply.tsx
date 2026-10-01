import { useEffect, useState } from 'react'
import { useAuiState } from '@assistant-ui/react'
import { Loader2Icon } from 'lucide-react'

export default function PendingReply({ label }: { label: string }) {
  const waiting = useAuiState(s => s.message.status?.type === 'running' && s.message.content.length === 0)
  const [seconds, setSeconds] = useState(0)

  useEffect(() => {
    if (!waiting) return
    const start = Date.now()
    const id = setInterval(() => setSeconds(Math.floor((Date.now() - start) / 1000)), 1000)
    return () => clearInterval(id)
  }, [waiting])

  if (!waiting) return null
  return (
    <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
      <Loader2Icon className="size-3.5 animate-spin" />
      <span>{label}</span>
      <span className="tabular-nums">{seconds}s</span>
    </div>
  )
}
