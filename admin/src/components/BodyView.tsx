import { formatBytes } from '@/lib/requestLog'

interface Binary {
  content_type: string | null
  bytes: number
  base64: string
}

interface UploadedFile extends Binary {
  field: string
  file_name: string
}

interface Multipart {
  fields: [string, string][]
  files: UploadedFile[]
}

interface ImagesResponse {
  data: { b64_json?: string; url?: string }[]
}

function isBinary(value: unknown): value is Binary {
  return typeof value === 'object' && value !== null && 'base64' in value && 'bytes' in value
}

function isMultipart(value: unknown): value is Multipart {
  return typeof value === 'object' && value !== null && 'files' in value && 'fields' in value
}

function isImages(value: unknown): value is ImagesResponse {
  return typeof value === 'object' && value !== null && Array.isArray((value as ImagesResponse).data)
    && (value as ImagesResponse).data.some(d => d.b64_json || d.url)
}

function dataUrl(contentType: string | null, base64: string): string {
  return `data:${contentType ?? 'application/octet-stream'};base64,${base64}`
}

function Media({ file, label }: { file: Binary; label: string }) {
  const type = file.content_type ?? ''
  const src = dataUrl(file.content_type, file.base64)
  return (
    <div className="border border-border bg-surface p-2 space-y-2">
      <div className="flex items-center justify-between gap-3 font-mono text-[11px] text-muted-foreground">
        <span className="text-foreground">{label}</span>
        <span>{type || 'binary'} · {formatBytes(file.bytes)}</span>
        <a href={src} download={label} className="text-brand hover:underline">download</a>
      </div>
      {type.startsWith('audio/') && <audio controls src={src} className="w-full" />}
      {type.startsWith('image/') && <img src={src} alt={label} className="max-h-96 border border-border" />}
    </div>
  )
}

export function stripPayloads(value: unknown): unknown {
  if (isBinary(value)) return { ...value, base64: `<${formatBytes(value.bytes)}>` }
  if (isMultipart(value)) return { ...value, files: value.files.map(f => stripPayloads(f)) }
  if (isImages(value)) {
    return {
      ...value,
      data: value.data.map(d => (d.b64_json ? { ...d, b64_json: `<${d.b64_json.length} base64 chars>` } : d)),
    }
  }
  return value
}

export function BodyMedia({ value }: { value: unknown }) {
  if (isBinary(value)) return <Media file={value} label="response" />
  if (isMultipart(value)) {
    return (
      <div className="space-y-2">
        {value.files.map((f, i) => <Media key={i} file={f} label={`${f.field}: ${f.file_name}`} />)}
      </div>
    )
  }
  if (isImages(value)) {
    return (
      <div className="flex flex-wrap gap-2">
        {value.data.map((d, i) => (
          <img
            key={i}
            src={d.b64_json ? dataUrl('image/png', d.b64_json) : d.url}
            alt={`image ${i + 1}`}
            className="max-h-96 border border-border"
          />
        ))}
      </div>
    )
  }
  return null
}
