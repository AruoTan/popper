import type { ComponentProps, JSX } from 'react'
import { lazy, Suspense } from 'react'
import {
  AlignLeft,
  ClipboardCheck,
  CircleHelp,
  ClipboardCopy,
  Copy,
  FileQuestion,
  Globe,
  Languages,
  Lightbulb,
  MessageCircleQuestion,
  Quote,
  ScanText,
  ScanSearch,
  Search,
  Sparkles,
  WandSparkles,
  type LucideIcon
} from 'lucide-react'

const STATIC_ICONS: Readonly<Record<string, LucideIcon>> = {
  'align-left': AlignLeft,
  'clipboard-check': ClipboardCheck,
  'clipboard-copy': ClipboardCopy,
  copy: Copy,
  'file-question': FileQuestion,
  globe: Globe,
  languages: Languages,
  lightbulb: Lightbulb,
  'message-circle-question': MessageCircleQuestion,
  quote: Quote,
  'scan-text': ScanText,
  'scan-search': ScanSearch,
  search: Search,
  sparkles: Sparkles,
  'wand-sparkles': WandSparkles,
  wand: WandSparkles
}

const DynamicActionIcon = lazy(async () => {
  const module = await import('./DynamicActionIcon')
  return { default: module.DynamicActionIcon }
})

function isSafeIconName(name: string): boolean {
  return /^[a-z0-9]+(?:-[a-z0-9]+)*$/u.test(name)
}

interface ActionIconProps extends Omit<ComponentProps<LucideIcon>, 'ref'> {
  name: string
}

export function ActionIcon({ name, ...props }: ActionIconProps): JSX.Element {
  const StaticIcon = STATIC_ICONS[name]
  if (StaticIcon) return <StaticIcon aria-hidden="true" {...props} />
  if (!isSafeIconName(name)) return <CircleHelp aria-hidden="true" {...props} />

  return (
    <Suspense fallback={<CircleHelp aria-hidden="true" {...props} />}>
      <DynamicActionIcon name={name} {...props} />
    </Suspense>
  )
}
