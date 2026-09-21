import { memo, useMemo, type JSX } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeKatex from 'rehype-katex'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'

import 'katex/dist/katex.min.css'

import { isSafeExternalUrl } from '../../shared'
import { contentLooksLikeMath } from './markdownPolicy'
import { normalizeMarkdownSource } from './markdownNormalize'

export interface SafeMarkdownProps {
  content: string
  onOpenExternal(url: string): void
}

// Stable plugin identity so react-markdown can skip re-setup when only text changes.
const REMARK_GFM_ONLY = [remarkGfm]
const REMARK_WITH_MATH = [remarkGfm, remarkMath]
const REHYPE_KATEX: NonNullable<React.ComponentProps<typeof ReactMarkdown>['rehypePlugins']> = [[
  rehypeKatex,
  {
    output: 'htmlAndMathml',
    strict: 'ignore',
    trust: false,
    maxSize: 20,
    maxExpand: 1_000
  }
]]

type MarkdownComponents = NonNullable<React.ComponentProps<typeof ReactMarkdown>['components']>

function createMarkdownComponents(onOpenExternal: (url: string) => void): MarkdownComponents {
  return {
    a: ({ href, children }) =>
      href && isSafeExternalUrl(href) ? (
        <a
          href={href}
          rel="noreferrer noopener"
          onClick={(event) => {
            event.preventDefault()
            if (
              event.button !== 0 ||
              event.metaKey ||
              event.ctrlKey ||
              event.shiftKey ||
              event.altKey
            ) {
              return
            }
            onOpenExternal(href)
          }}
          onAuxClick={(event) => {
            event.preventDefault()
          }}
        >
          {children}
        </a>
      ) : (
        <span>{children}</span>
      ),
    img: ({ alt }) => <span>{alt ? `[图片：${alt}]` : '[图片]'}</span>
  }
}

/**
 * Renders model output without raw HTML or automatic remote image requests.
 * Links are handed to the main process only after an explicit HTTP(S) check.
 * KaTeX plugins run only when content looks like math (conservative heuristic).
 *
 * Source is normalized once (memoized) so translation paragraphs / Chinese
 * list markers survive CommonMark's single-newline collapsing.
 */
function SafeMarkdownView({ content, onOpenExternal }: SafeMarkdownProps): JSX.Element {
  const normalized = useMemo(() => normalizeMarkdownSource(content), [content])
  const enableMath = useMemo(() => contentLooksLikeMath(normalized), [normalized])
  const components = useMemo(
    () => createMarkdownComponents(onOpenExternal),
    [onOpenExternal]
  )

  return (
    <ReactMarkdown
      skipHtml
      remarkPlugins={enableMath ? REMARK_WITH_MATH : REMARK_GFM_ONLY}
      rehypePlugins={enableMath ? REHYPE_KATEX : undefined}
      components={components}
    >
      {normalized}
    </ReactMarkdown>
  )
}

export const SafeMarkdown = memo(SafeMarkdownView)
