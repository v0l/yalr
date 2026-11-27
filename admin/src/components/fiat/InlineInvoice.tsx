import { useState } from 'react'
import { QRCodeSVG } from 'qrcode.react'
import { CheckIcon, CopyIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'

export default function InlineInvoice({ bolt11, amountSats }: { bolt11: string; amountSats: number }) {
  const [copied, setCopied] = useState(false)

  async function copy() {
    await navigator.clipboard.writeText(bolt11)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Pay with Lightning</span>
        <span className="font-mono text-[12px] tabular-nums text-brand">{amountSats.toLocaleString()} sats</span>
      </div>
      <div className="flex justify-center">
        <a href={`lightning:${bolt11}`} className="bg-white p-3">
          <QRCodeSVG value={bolt11.toUpperCase()} size={208} />
        </a>
      </div>
      <div className="relative">
        <div className="max-h-24 overflow-y-auto break-all border border-border bg-surface p-3 pr-12 font-mono text-[11px] leading-relaxed text-brand">{bolt11}</div>
        <Button variant="ghost" size="icon" onClick={copy} className="absolute right-1 top-1/2 size-8 -translate-y-1/2 text-muted-foreground hover:text-foreground" title="Copy invoice">
          {copied ? <CheckIcon className="size-3.5 text-brand" /> : <CopyIcon className="size-3.5" />}
        </Button>
      </div>
    </div>
  )
}
