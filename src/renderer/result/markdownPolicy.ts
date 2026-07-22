export const RICH_MARKDOWN_SCALAR_LIMIT = 16_384

const BLOCK_MARKDOWN = /(?:^|\n)[ \t]{0,3}(?:#{1,6}[ \t]+|>[ \t]+|[-+*][ \t]+|\d+[.)][ \t]+|```|~~~|\$\$|\|[^\n]*\|)/u
const INLINE_MARKDOWN = /(?:\[[^\]\n]+\]\([^\n)]+\)|`[^`\n]+`|\*\*[^*\n]+\*\*|__[^_\n]+__|\$[^$\n]+\$)/u

export function hasLikelyMarkdownSyntax(content: string): boolean {
  return BLOCK_MARKDOWN.test(content) || INLINE_MARKDOWN.test(content)
}

export function shouldRenderRichMarkdown(
  content: string,
  contentScalarCount: number
): boolean {
  return content.length > 0 &&
    contentScalarCount <= RICH_MARKDOWN_SCALAR_LIMIT &&
    hasLikelyMarkdownSyntax(content)
}
