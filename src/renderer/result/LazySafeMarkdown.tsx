import { lazy } from 'react'

export type SafeMarkdownModule = typeof import('./SafeMarkdown')
export type SafeMarkdownImporter = () => Promise<SafeMarkdownModule>

export interface MarkdownGeneration {
  id: number
  Component: React.LazyExoticComponent<
    React.ComponentType<import('./SafeMarkdown').SafeMarkdownProps>
  >
}

export interface RetryableMarkdownLoader {
  preload(): Promise<void>
  nextGeneration(): MarkdownGeneration
}

export function createRetryableMarkdownLoader(
  importer: SafeMarkdownImporter = () => import('./SafeMarkdown')
): RetryableMarkdownLoader {
  let resolved: SafeMarkdownModule | null = null
  let inFlight: Promise<SafeMarkdownModule> | null = null
  let generation = 0

  const load = (): Promise<SafeMarkdownModule> => {
    if (resolved) return Promise.resolve(resolved)
    if (inFlight) return inFlight
    inFlight = importer().then(
      (module) => {
        resolved = module
        return module
      },
      (error: unknown) => {
        inFlight = null
        throw error
      }
    )
    return inFlight
  }

  return {
    preload: async () => { await load() },
    nextGeneration: () => ({
      id: ++generation,
      Component: lazy(async () => ({ default: (await load()).SafeMarkdown }))
    })
  }
}

export const safeMarkdownLoader = createRetryableMarkdownLoader()
