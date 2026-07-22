import type { ComponentProps, JSX, LazyExoticComponent } from 'react'
import { lazy, Suspense } from 'react'
import { CircleHelp, type LucideIcon } from 'lucide-react'

import { getDynamicIconImporter, isLucideIconName } from './lucideIconRegistry'

const dynamicCache = new Map<string, LazyExoticComponent<LucideIcon>>()

interface DynamicActionIconProps extends Omit<ComponentProps<LucideIcon>, 'ref'> {
  name: string
}

function dynamicIcon(name: string): LazyExoticComponent<LucideIcon> | null {
  if (!isLucideIconName(name)) return null
  const cached = dynamicCache.get(name)
  if (cached) return cached
  const Icon = lazy(getDynamicIconImporter(name))
  dynamicCache.set(name, Icon)
  return Icon
}

export function DynamicActionIcon({ name, ...props }: DynamicActionIconProps): JSX.Element {
  const Icon = dynamicIcon(name)
  if (!Icon) return <CircleHelp aria-hidden="true" {...props} />

  return (
    <Suspense fallback={<CircleHelp aria-hidden="true" {...props} />}>
      <Icon aria-hidden="true" {...props} />
    </Suspense>
  )
}
