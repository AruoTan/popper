export function countUnicodeScalars(value: string): number {
  let count = 0
  for (const _scalar of value) count += 1
  return count
}

export function hasAtMostUnicodeScalars(value: string, limit: number): boolean {
  if (!Number.isSafeInteger(limit) || limit < 0) {
    throw new RangeError('Unicode scalar limit must be a non-negative safe integer')
  }
  let count = 0
  for (const _scalar of value) {
    count += 1
    if (count > limit) return false
  }
  return true
}
