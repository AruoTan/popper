import type { JSX } from 'react';

export function CaptureMouseIcon({ action }: { action: 'right' | 'wheel-up' | 'wheel-down' }): JSX.Element {
  const scrolling = action !== 'right';
  return <svg width={scrolling ? 30 : 22} height="24" viewBox={`0 0 ${scrolling ? 32 : 24} 26`} fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" role="img" aria-label={action === 'right' ? '鼠标右键' : action === 'wheel-up' ? '滚轮向上' : '滚轮向下'}>
    {action === 'right' && <path d="M12 2a7 7 0 0 1 7 7v2h-7Z" fill="currentColor" stroke="none" />}
    <rect x="4" y="1" width="16" height="24" rx="8" opacity=".7" />
    {action === 'right' ? <path d="M4 11h16M12 1v10" opacity=".7" /> : <>
      <path d="M12 5v5" strokeWidth="2.5" />
      <path d={action === 'wheel-up' ? 'M24 10l3-3 3 3M27 7v12' : 'M24 16l3 3 3-3M27 7v12'} />
    </>}
  </svg>;
}
