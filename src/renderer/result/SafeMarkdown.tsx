import { memo, useMemo, type JSX } from 'react'
import ReactMarkdown, { defaultUrlTransform } from 'react-markdown'
import rehypeKatex from 'rehype-katex'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'

function safeUrlTransform(url: string): string {
  if (url.startsWith('attachment:') || url.startsWith('data:image/')) {
    return url
  }
  return defaultUrlTransform(url)
}

import 'katex/dist/katex.min.css'

import { isSafeExternalUrl } from '../../shared'
import { contentLooksLikeMath } from './markdownPolicy'
import { normalizeMarkdownSource } from './markdownNormalize'

export interface SafeMarkdownProps {
  content: string
  attachments?: ReadonlyMap<string, { name: string; mime: string; kind: string; data: string }>
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

function createMarkdownComponents(
  onOpenExternal: (url: string) => void,
  attachments?: ReadonlyMap<string, { name: string; mime: string; kind: string; data: string }>
): MarkdownComponents {
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
    img: ({ alt, src }) => {
      if (src && src.startsWith('data:image/')) {
        return <img src={src} alt={alt || ''} className="result-markdown-image" loading="lazy" />
      }
      const lookupKey = (src && src.startsWith('attachment:') ? src.slice('attachment:'.length) : src) || alt || ''
      const matched = attachments?.get(lookupKey) || (alt ? attachments?.get(alt) : undefined)
      if (matched && matched.kind === 'image' && matched.data) {
        return (
          <img
            src={`data:${matched.mime || 'image/png'};base64,${matched.data}`}
            alt={alt || matched.name}
            className="result-markdown-image"
            loading="lazy"
          />
        )
      }
      return <span>{alt ? `[图片：${alt}]` : '[图片]'}</span>
    }
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
function SafeMarkdownView({ content, attachments, onOpenExternal }: SafeMarkdownProps): JSX.Element {
  const normalized = useMemo(() => {
    let text = normalizeMarkdownSource(content)
    if (attachments && attachments.size > 0) {
      text = text.replace(/\[图片[：:]\s*([^\]]+)\]/gu, (match, filename: string) => {
        const trimmed = filename.trim()
        if (attachments.has(trimmed)) {
          return `![${trimmed}](attachment:${trimmed})`
        }
        return match
      })
    }
    return text
  }, [content, attachments])
  const enableMath = useMemo(() => contentLooksLikeMath(normalized), [normalized])
  const components = useMemo(
    () => createMarkdownComponents(onOpenExternal, attachments),
    [onOpenExternal, attachments]
  )

  return (
    <ReactMarkdown
      skipHtml
      urlTransform={safeUrlTransform}
      remarkPlugins={enableMath ? REMARK_WITH_MATH : REMARK_GFM_ONLY}
      rehypePlugins={enableMath ? REHYPE_KATEX : undefined}
      components={components}
    >
      {normalized}
    </ReactMarkdown>
  )
}

export const SafeMarkdown = memo(SafeMarkdownView)
