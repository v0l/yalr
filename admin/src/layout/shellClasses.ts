export function sidebarClass(open: boolean): string {
  return [
    'fixed inset-y-0 left-0 z-40 flex w-56 flex-col bg-sidebar border-r border-border/50',
    'transition-transform md:translate-x-0',
    open ? 'translate-x-0' : '-translate-x-full',
  ].join(' ')
}

export const mainClass = 'flex-1 min-w-0 md:ml-56 min-h-screen bg-background'
export const topBarClass =
  'md:hidden sticky top-0 z-30 flex items-center gap-3 h-12 px-3 bg-sidebar border-b border-border/50'
export const shellClass = 'min-h-screen flex flex-col md:flex-row bg-background overflow-x-hidden'
