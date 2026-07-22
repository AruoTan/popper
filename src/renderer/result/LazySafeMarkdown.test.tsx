import type { JSX } from 'react'

import type { SafeMarkdownProps } from './SafeMarkdown'
import { createRetryableMarkdownLoader, type SafeMarkdownImporter } from './LazySafeMarkdown'

describe('retryable Markdown loader', () => {
  it('clears a rejected import and creates a new lazy type for retry', async () => {
    const FakeSafeMarkdown = ({ content }: SafeMarkdownProps): JSX.Element => <span>{content}</span>
    const importer = vi.fn()
      .mockRejectedValueOnce(new Error('first import failed'))
      .mockResolvedValueOnce({ SafeMarkdown: FakeSafeMarkdown })
    const loader = createRetryableMarkdownLoader(importer as unknown as SafeMarkdownImporter)
    const first = loader.nextGeneration()

    await expect(loader.preload()).rejects.toThrow('first import failed')
    const second = loader.nextGeneration()
    expect(second.id).toBe(first.id + 1)
    expect(second.Component).not.toBe(first.Component)
    await expect(loader.preload()).resolves.toBeUndefined()
    expect(importer).toHaveBeenCalledTimes(2)
  })

  it('shares a concurrent import and reuses the resolved module across generations', async () => {
    let resolveImport!: (module: { SafeMarkdown: (props: SafeMarkdownProps) => JSX.Element }) => void
    const importPromise = new Promise<{ SafeMarkdown: (props: SafeMarkdownProps) => JSX.Element }>(
      (resolve) => { resolveImport = resolve }
    )
    const importer = vi.fn(() => importPromise)
    const loader = createRetryableMarkdownLoader(importer as unknown as SafeMarkdownImporter)

    const firstPreload = loader.preload()
    const secondPreload = loader.preload()
    expect(importer).toHaveBeenCalledOnce()

    resolveImport({ SafeMarkdown: ({ content }) => <span>{content}</span> })
    await expect(Promise.all([firstPreload, secondPreload])).resolves.toEqual([undefined, undefined])

    const first = loader.nextGeneration()
    const second = loader.nextGeneration()
    expect(second.id).toBe(first.id + 1)
    expect(second.Component).not.toBe(first.Component)
    await expect(loader.preload()).resolves.toBeUndefined()
    expect(importer).toHaveBeenCalledOnce()
  })
})
