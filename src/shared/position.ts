import {
  TOOLBAR_SCREEN_GAP,
  TOOLBAR_SCREEN_MARGIN
} from './constants'
import type { Point, Rectangle, SelectionAnchor, ToolbarSize } from './schemas'

export interface DisplayWorkArea {
  id: number | string
  workArea: Rectangle
}

export interface ToolbarPosition {
  x: number
  y: number
  displayId: number | string
  placement: 'below' | 'above' | 'clamped'
}

export interface ToolbarPositionOptions {
  anchor?: SelectionAnchor | null
  cursor?: Point | null
  displays: readonly DisplayWorkArea[]
  toolbarSize: ToolbarSize
  gap?: number
  margin?: number
}

interface ResolvedAnchor {
  point: Point
  left: number
  right: number
  top: number
  bottom: number
}

function isFinitePoint(value: Point | null | undefined): value is Point {
  return Boolean(value && Number.isFinite(value.x) && Number.isFinite(value.y))
}

function resolveAnchor(anchor: SelectionAnchor | null | undefined, cursor: Point | null | undefined): ResolvedAnchor {
  if (anchor?.kind === 'selection') {
    const values = [anchor.x, anchor.y, anchor.width, anchor.height]
    if (values.every(Number.isFinite) && anchor.width >= 0 && anchor.height >= 0) {
      return {
        point: { x: anchor.x + anchor.width / 2, y: anchor.y + anchor.height / 2 },
        left: anchor.x,
        right: anchor.x + anchor.width,
        top: anchor.y,
        bottom: anchor.y + anchor.height
      }
    }
  }

  if (anchor?.kind === 'cursor' && isFinitePoint(anchor)) {
    return {
      point: anchor,
      left: anchor.x,
      right: anchor.x,
      top: anchor.y,
      bottom: anchor.y
    }
  }

  if (isFinitePoint(cursor)) {
    return {
      point: cursor,
      left: cursor.x,
      right: cursor.x,
      top: cursor.y,
      bottom: cursor.y
    }
  }

  const first = cursor ?? { x: 0, y: 0 }
  return { point: first, left: first.x, right: first.x, top: first.y, bottom: first.y }
}

function distanceToRectangle(point: Point, rectangle: Rectangle): number {
  const dx = Math.max(rectangle.x - point.x, 0, point.x - (rectangle.x + rectangle.width))
  const dy = Math.max(rectangle.y - point.y, 0, point.y - (rectangle.y + rectangle.height))
  return dx * dx + dy * dy
}

function displayForPoint(point: Point, displays: readonly DisplayWorkArea[]): DisplayWorkArea {
  const containing = displays.find(({ workArea }) =>
    point.x >= workArea.x &&
    point.x <= workArea.x + workArea.width &&
    point.y >= workArea.y &&
    point.y <= workArea.y + workArea.height
  )
  if (containing) return containing

  return displays.reduce((nearest, display) =>
    distanceToRectangle(point, display.workArea) < distanceToRectangle(point, nearest.workArea)
      ? display
      : nearest
  )
}

function clamp(value: number, minimum: number, maximum: number): number {
  if (maximum < minimum) return minimum
  return Math.min(Math.max(value, minimum), maximum)
}

export function calculateToolbarPosition(options: ToolbarPositionOptions): ToolbarPosition {
  if (options.displays.length === 0) {
    throw new RangeError('至少需要一个显示器工作区')
  }
  if (!(options.toolbarSize.width > 0) || !(options.toolbarSize.height > 0)) {
    throw new RangeError('工具栏尺寸必须大于零')
  }

  const gap = Math.max(0, options.gap ?? TOOLBAR_SCREEN_GAP)
  const margin = Math.max(0, options.margin ?? TOOLBAR_SCREEN_MARGIN)
  const anchor = resolveAnchor(options.anchor, options.cursor)
  const display = displayForPoint(anchor.point, options.displays)
  const area = display.workArea

  const minX = area.x + margin
  const maxX = area.x + area.width - options.toolbarSize.width - margin
  const minY = area.y + margin
  const maxY = area.y + area.height - options.toolbarSize.height - margin

  const desiredX = (anchor.left + anchor.right - options.toolbarSize.width) / 2
  const belowY = anchor.bottom + gap
  const aboveY = anchor.top - gap - options.toolbarSize.height

  let desiredY: number
  let placement: ToolbarPosition['placement']
  if (belowY <= maxY) {
    desiredY = belowY
    placement = 'below'
  } else if (aboveY >= minY) {
    desiredY = aboveY
    placement = 'above'
  } else {
    desiredY = belowY
    placement = 'clamped'
  }

  return {
    x: Math.round(clamp(desiredX, minX, maxX)),
    y: Math.round(clamp(desiredY, minY, maxY)),
    displayId: display.id,
    placement
  }
}

