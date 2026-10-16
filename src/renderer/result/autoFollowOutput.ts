import { useCallback, type RefObject, type UIEvent } from 'react'

const BOTTOM_THRESHOLD_PX = 64

export function isNearScrollBottom(element: HTMLElement): boolean {
  return element.scrollHeight - element.scrollTop - element.clientHeight < BOTTOM_THRESHOLD_PX
}

/**
 * 滚动条控制钩子：
 * 保持滑动条始终由用户掌控。AI 流式输出时不主动修改 scrollTop，
 * 不带动滑动条变化，也不将滑动条固定在底部或某一处。
 */
export function useAutoFollowOutput(
  _scrollRef?: RefObject<HTMLDivElement | null>,
  _observedRef?: RefObject<HTMLDivElement | null>,
  _resetKey?: string | null,
  _enabled = true
): (event: UIEvent<HTMLDivElement>) => void {
  return useCallback((_event: UIEvent<HTMLDivElement>): void => {
    // 滚动位置始终由用户操作驱动，不被自动逻辑干扰
  }, [])
}

