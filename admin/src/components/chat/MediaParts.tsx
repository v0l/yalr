import { AttachmentPrimitive, useAuiState, type ImageMessagePartComponent, type FileMessagePartComponent } from '@assistant-ui/react'
import { useEffect, useMemo } from 'react'
import { DownloadIcon, ExternalLinkIcon, FileIcon, FileAudioIcon, XIcon } from 'lucide-react'

function toObjectUrl(src: string): string {
  const match = /^data:([^;,]+);base64,(.*)$/s.exec(src)
  if (!match) return src
  const bytes = Uint8Array.from(atob(match[2]), c => c.charCodeAt(0))
  return URL.createObjectURL(new Blob([bytes], { type: match[1] }))
}

function shortHash(text: string) {
  let hash = 0x811c9dc5
  for (let i = 0; i < text.length; i++) hash = Math.imul(hash ^ text.charCodeAt(i), 0x01000193)
  return (hash >>> 0).toString(16).padStart(8, '0')
}

function extensionFor(src: string) {
  return /^data:image\/(\w+)/.exec(src)?.[1]?.replace('jpeg', 'jpg') ?? 'png'
}

const imageAction = 'inline-flex items-center gap-1 border border-border bg-card px-2 py-0.5 text-[11px] uppercase tracking-wider text-muted-foreground transition-colors hover:bg-surface hover:text-foreground'

export const ImagePart: ImageMessagePartComponent = ({ image, filename }) => {
  const url = useMemo(() => toObjectUrl(image), [image])
  useEffect(() => () => { if (url !== image) URL.revokeObjectURL(url) }, [url, image])
  const name = filename ?? `yalr-image-${shortHash(image)}.${extensionFor(image)}`

  return (
    <div className="my-2 flex w-fit flex-col gap-1.5">
      <a href={url} target="_blank" rel="noreferrer" className="block border border-border bg-card p-1" title="Open in new tab">
        <img src={url} alt={filename ?? 'Generated image'} className="max-h-96 max-w-full object-contain" />
      </a>
      <div className="flex gap-1.5">
        <a href={url} target="_blank" rel="noreferrer" className={imageAction}>
          <ExternalLinkIcon className="size-3" /> Open
        </a>
        <a href={url} download={name} className={imageAction}>
          <DownloadIcon className="size-3" /> Download
        </a>
      </div>
    </div>
  )
}

export const FilePart: FileMessagePartComponent = ({ data, mimeType, filename }) => {
  if (mimeType.startsWith('audio/')) {
    return <audio controls src={data} className="my-2 h-9 w-80 max-w-full" />
  }
  return (
    <a href={data} download={filename} className="my-2 inline-flex items-center gap-2 border border-border bg-card px-2 py-1 text-[12px] text-foreground hover:bg-surface">
      <FileIcon className="size-3.5" /> {filename ?? mimeType}
    </a>
  )
}

function AttachmentPreview() {
  const attachment = useAuiState(s => s.attachment)
  const image = attachment?.content?.find(p => p.type === 'image')
  const preview = image?.type === 'image' ? image.image : attachment?.file && attachment.type === 'image' ? URL.createObjectURL(attachment.file) : null
  if (preview) return <img src={preview} alt="" className="size-8 object-cover" />
  const Icon = attachment?.contentType?.startsWith('audio/') ? FileAudioIcon : FileIcon
  return <Icon className="size-4 text-muted-foreground" />
}

export function ComposerAttachment() {
  return (
    <AttachmentPrimitive.Root className="flex h-10 items-center gap-2 border border-border bg-surface pl-1 pr-1.5 text-[11px] text-foreground">
      <AttachmentPreview />
      <span className="max-w-32 truncate"><AttachmentPrimitive.Name /></span>
      <AttachmentPrimitive.Remove className="p-0.5 text-muted-foreground hover:text-destructive" title="Remove">
        <XIcon className="size-3" />
      </AttachmentPrimitive.Remove>
    </AttachmentPrimitive.Root>
  )
}

export function MessageAttachment() {
  return (
    <AttachmentPrimitive.Root className="flex h-10 items-center gap-2 border border-border bg-card pl-1 pr-2 text-[11px] text-muted-foreground">
      <AttachmentPreview />
      <span className="max-w-40 truncate"><AttachmentPrimitive.Name /></span>
    </AttachmentPrimitive.Root>
  )
}
