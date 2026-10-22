import { cn } from '@/lib/utils'
import type { ListingStatus } from '@/api/market'

const OPTIONS = [
  {
    value: true,
    title: 'For sale',
    detail: 'Shown on the public price list at /account and announced to Routstr. Anyone with a funded key can call it.',
  },
  {
    value: false,
    title: 'Not for sale',
    detail: 'Left off the price list and the Routstr announcement. Regular paying users cannot call it. Admins, billing exempt users and users with an explicit model permission still can.',
  },
]

export default function AdvertiseField({
  value,
  onChange,
  listing,
}: {
  value: boolean
  onChange: (value: boolean) => void
  listing: ListingStatus | null
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Availability</span>
      <div className="grid grid-cols-2 gap-2">
        {OPTIONS.map(option => (
          <button
            key={option.title}
            type="button"
            onClick={() => onChange(option.value)}
            className={cn(
              'flex flex-col gap-1 border p-3 text-left transition-colors',
              value === option.value ? 'border-brand/40 bg-brand/10' : 'border-border bg-surface hover:border-brand/20',
            )}
          >
            <span className={cn('font-mono text-[11px] uppercase tracking-wider', value === option.value ? 'text-brand' : 'text-foreground')}>
              {option.title}
            </span>
            <span className="font-mono text-[10px] leading-relaxed text-muted-foreground">{option.detail}</span>
          </button>
        ))}
      </div>
      <p className="font-mono text-[10px] text-muted-foreground/80">
        {!listing
          ? ''
          : listing.discovery_enabled
            ? `Routstr announcement: ${listing.provider_id} on ${listing.relays.length} relays.`
            : 'Routstr discovery is off in the server config, so this only controls the public price list.'}
      </p>
    </div>
  )
}
