import type { Provider } from '../types'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'

export interface ProviderDeleteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  target: Provider | null
  onDelete: () => void
  deleting: boolean
}

export function ProviderDeleteDialog({ open, onOpenChange, target, onDelete, deleting }: ProviderDeleteDialogProps) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent className="border-border bg-card">
        <AlertDialogHeader>
          <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DELETE PROVIDER</AlertDialogTitle>
          <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">
            Delete <span className="text-foreground">{target?.name}</span>? This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={onDelete} disabled={deleting} className="font-mono text-[12px] tracking-wider uppercase">
            {deleting ? 'DELETING...' : 'DELETE'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
