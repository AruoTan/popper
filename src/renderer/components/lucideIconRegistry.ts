import dynamicIconImports from 'lucide-react/dynamicIconImports'
import type { LucideIcon } from 'lucide-react'

export type DynamicIconName = keyof typeof dynamicIconImports

const dynamicIconNames = new Set<string>(Object.keys(dynamicIconImports))

export const LUCIDE_ICON_NAMES: readonly string[] = Object.freeze(
  [...dynamicIconNames].sort((left, right) => left.localeCompare(right))
)

export function isLucideIconName(name: string): name is DynamicIconName {
  return /^[a-z0-9]+(?:-[a-z0-9]+)*$/u.test(name) && dynamicIconNames.has(name)
}

export function getDynamicIconImporter(
  name: DynamicIconName
): () => Promise<{ default: LucideIcon }> {
  return dynamicIconImports[name] as () => Promise<{ default: LucideIcon }>
}
