import {
  RICH_MARKDOWN_SCALAR_LIMIT,
  hasLikelyMarkdownSyntax,
  shouldRenderRichMarkdown
} from './markdownPolicy'

describe('rich Markdown policy', () => {
  it.each([
    'ordinary plain text',
    'a sentence with a hyphen - in the middle',
    'price is $5 without a closing math delimiter'
  ])('keeps obvious plain text lightweight: %s', (content) => {
    expect(hasLikelyMarkdownSyntax(content)).toBe(false)
  })

  it.each([
    '# heading',
    '- list item',
    '[link](https://example.com)',
    '```ts\nconst value = 1\n```',
    '$x^2$',
    '| A | B |\n| - | - |'
  ])('recognizes representative Markdown: %s', (content) => {
    expect(hasLikelyMarkdownSyntax(content)).toBe(true)
  })

  it('uses Unicode scalars at the exact rich-render boundary', () => {
    const atLimit = `# ${'😀'.repeat(RICH_MARKDOWN_SCALAR_LIMIT - 2)}`
    expect(shouldRenderRichMarkdown(atLimit, RICH_MARKDOWN_SCALAR_LIMIT)).toBe(true)
    expect(shouldRenderRichMarkdown(`${atLimit}😀`, RICH_MARKDOWN_SCALAR_LIMIT + 1)).toBe(false)
  })
})
