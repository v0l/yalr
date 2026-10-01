import { AttachmentPrimitive, useAuiState, type ImageMessagePartComponent, type FileMessagePartComponent } from '@assistant-ui/react'
import { FileIcon, FileAudioIcon, XIcon } from 'lucide-react'

export const ImagePart: ImageMessagePartComponent = ({ image, filename }) => (
  <a href={image} target="_blank" rel="noreferrer" className="my-2 block w-fit border border-border bg-card p-1">
    <img src={image} alt={filename ?? 'Generated image'} className="max-h-96 max-w-full object-contain" />
  </a>
)

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
