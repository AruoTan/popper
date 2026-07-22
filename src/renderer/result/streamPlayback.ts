export const MAX_GRAPHEME_CARRY_SCALARS = 256

export interface GraphemeTailUpdate {
  released: string
  carry: string
}

let segmenter: Intl.Segmenter | null | undefined

export function splitGraphemes(value: string): string[] {
  if (typeof Intl.Segmenter === 'function') {
    segmenter ??= new Intl.Segmenter(undefined, { granularity: 'grapheme' })
    return Array.from(segmenter.segment(value), (part) => part.segment)
  }
  return Array.from(value)
}

function boundedTailStart(value: string, limit: number): number {
  let index = value.length
  let scalarCount = 0
  while (index > 0 && scalarCount < limit) {
    index -= 1
    const codeUnit = value.charCodeAt(index)
    if (
      codeUnit >= 0xdc00 &&
      codeUnit <= 0xdfff &&
      index > 0
    ) {
      const preceding = value.charCodeAt(index - 1)
      if (preceding >= 0xd800 && preceding <= 0xdbff) index -= 1
    }
    scalarCount += 1
  }
  return index
}

export function splitStableGraphemeTail(
  previousCarry: string,
  delta: string,
  segment: (value: string) => readonly string[] = splitGraphemes
): GraphemeTailUpdate {
  const previousCarryStart = boundedTailStart(previousCarry, MAX_GRAPHEME_CARRY_SCALARS)
  const deltaTailStart = boundedTailStart(delta, MAX_GRAPHEME_CARRY_SCALARS)
  const stablePrefix = previousCarry.slice(0, previousCarryStart) + delta.slice(0, deltaTailStart)
  const tail = previousCarry.slice(previousCarryStart) + delta.slice(deltaTailStart)
  const graphemes = segment(tail)
  const lastGrapheme = graphemes.at(-1) ?? ''
  let released = stablePrefix + graphemes.slice(0, -1).join('')
  const carryStart = boundedTailStart(lastGrapheme, MAX_GRAPHEME_CARRY_SCALARS)
  if (carryStart > 0) released += lastGrapheme.slice(0, carryStart)

  return { released, carry: lastGrapheme.slice(carryStart) }
}

export function flushGraphemeTail(carry: string): string {
  return carry
}
