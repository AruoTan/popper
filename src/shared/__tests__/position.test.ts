import { describe, expect, it } from 'vitest'

import { calculateToolbarPosition, type DisplayWorkArea } from '..'

const primary: DisplayWorkArea = {
  id: 1,
  workArea: { x: 0, y: 0, width: 1_440, height: 900 }
}

describe('toolbar position', () => {
  it('centres below a selection in macOS logical coordinates', () => {
    expect(
      calculateToolbarPosition({
        anchor: { kind: 'selection', x: 600, y: 200, width: 240, height: 24 },
        displays: [primary],
        toolbarSize: { width: 360, height: 48 }
      })
    ).toEqual({ x: 540, y: 232, displayId: 1, placement: 'below' })
  })

  it('clamps the toolbar to the right screen edge', () => {
    const result = calculateToolbarPosition({
      anchor: { kind: 'selection', x: 1_400, y: 300, width: 30, height: 20 },
      displays: [primary],
      toolbarSize: { width: 360, height: 48 }
    })
    expect(result.x).toBe(1_072)
  })

  it('places the toolbar above a selection near the bottom', () => {
    const result = calculateToolbarPosition({
      anchor: { kind: 'selection', x: 600, y: 865, width: 100, height: 20 },
      displays: [primary],
      toolbarSize: { width: 360, height: 48 }
    })
    expect(result).toMatchObject({ y: 809, placement: 'above' })
  })

  it('selects and clamps against a negative-coordinate secondary monitor', () => {
    const left: DisplayWorkArea = {
      id: 2,
      workArea: { x: -1_920, y: -120, width: 1_920, height: 1_080 }
    }
    const result = calculateToolbarPosition({
      anchor: { kind: 'selection', x: -1_915, y: 100, width: 20, height: 20 },
      displays: [primary, left],
      toolbarSize: { width: 360, height: 48 }
    })
    expect(result.displayId).toBe(2)
    expect(result.x).toBe(-1_912)
    expect(result.y).toBe(128)
  })

  it('falls back to the cursor when selection coordinates are unavailable', () => {
    const left: DisplayWorkArea = {
      id: 'left',
      workArea: { x: -1_280, y: 0, width: 1_280, height: 800 }
    }
    const result = calculateToolbarPosition({
      cursor: { x: -640, y: 400 },
      displays: [primary, left],
      toolbarSize: { width: 300, height: 40 }
    })
    expect(result).toMatchObject({ x: -790, y: 408, displayId: 'left', placement: 'below' })
  })
})

