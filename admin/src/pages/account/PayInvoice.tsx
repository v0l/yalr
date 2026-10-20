import { useEffect, useRef, useState } from 'react'
import { QRCodeSVG } from 'qrcode.react'
import { CheckIcon, CopyIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { accountApi, type CreatedInvoice, type InvoiceStatus } from '@/api/account'

const POLL_MS = 3000

export default function PayInvoice({
  invoice,
  onPaid,
  onCancel,
}: {
  invoice: CreatedInvoice
  onPaid: (status: InvoiceStatus) => void
  onCancel: () => void
}) {
  const [copied, setCopied] = useState(false)
  const [state, setState] = useState<string>('pending')
  const paid = useRef(onPaid)
  paid.current = onPaid

  useEffect(() => {
    let stopped = false
    async function poll() {
      try {
        const status = await accountApi.invoiceStatus(invoice.invoice_id)
        if (stopped) return
        setState(status.status)
        if (status.status === 'paid') return paid.current(status)
        if (status.status === 'expired' || status.status === 'cancelled') return
      } catch {
        if (stopped) return
      }
      setTimeout(poll, POLL_MS)
    }
    poll()
    return () => { stopped = true }
  }, [invoice.invoice_id])

  async function copy() {
    await navigator.clipboard.writeText(invoice.bolt11)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  const expired = state === 'expired' || state === 'cancelled'

  return (
    <div className="panel space-y-4 p-4">
      <div className="flex items-center justify-between">
        <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Pay with Lightning</span>
        <span className="font-mono text-[12px] tabular-nums text-brand">{invoice.amount_sats.toLocaleString()} sats</span>
      </div>
      <div className="flex justify-center">
        <a href={`lightning:${invoice.bolt11}`} className={`bg-white p-3 ${expired ? 'opacity-30' : ''}`}>
          <QRCodeSVG value={invoice.bolt11.toUpperCase()} size={208} />
        </a>
      </div>
      <div className="relative">
        <div className="break-all border border-border bg-surface p-3 pr-12 font-mono text-[11px] leading-relaxed text-brand">{invoice.bolt11}</div>
        <Button variant="ghost" size="icon" onClick={copy} className="absolute right-1 top-1/2 size-8 -translate-y-1/2 text-muted-foreground hover:text-foreground" title="Copy invoice">
          {copied ? <CheckIcon className="size-3.5 text-brand" /> : <CopyIcon className="size-3.5" />}
        </Button>
      </div>
      <div className="flex items-center justify-between">
        <span className={`font-mono text-[11px] uppercase tracking-wider ${expired ? 'text-destructive' : 'text-muted-foreground'}`}>
          {expired ? 'Invoice expired' : <><span className="animate-pulse-status text-brand">▸</span> Waiting for payment</>}
        </span>
        <Button variant="outline" size="sm" onClick={onCancel} className="h-8 border-border font-mono text-[11px] uppercase tracking-wider text-muted-foreground">
          {expired ? 'Back' : 'Cancel'}
        </Button>
      </div>
    </div>
  )
}
