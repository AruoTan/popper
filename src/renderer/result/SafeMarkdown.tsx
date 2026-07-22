import { memo, type JSX } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeKatex from 'rehype-katex'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'

import 'katex/dist/katex.min.css'

import { isSafeExternalUrl } from '../../shared'

export interface SafeMarkdownProps {
  content: string
  onOpenExternal(url: string): void
}

const REMARK_PLUGINS = [remarkGfm, remarkMath]
const REHYPE_PLUGINS: NonNullable<React.ComponentProps<typeof ReactMarkdown>['rehypePlugins']> = [[
  rehypeKatex,
  {
    output: 'htmlAndMathml',
    strict: 'ignore',
    trust: false,
    maxSize: 20,
    maxExpand: 1_000
  }
]]

/**
 * Renders model output without raw HTML or automatic remote image requests.
 * Links are handed to the main process only after an explicit HTTP(S) check.
 */
function SafeMarkdownView({ content, onOpenExternal }: SafeMarkdownProps): JSX.Element {
  return (
    <ReactMarkdown
      skipHtml
      remarkPlugins={REMARK_PLUGINS}
      rehypePlugins={REHYPE_PLUGINS}
      components={{
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
      }}
    >
      {content}
    </ReactMarkdown>
  )
}

export const SafeMarkdown = memo(SafeMarkdownView)
