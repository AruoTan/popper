import { isValidTauriGlobalShortcut } from '../../shared'

/** Suggested default when switching into shortcut trigger mode with an empty field. */
export const SUGGESTED_CAPTURE_SHORTCUT = 'CommandOrControl+Shift+S'

export type ShortcutKeyLike = Pick<
  KeyboardEvent,
  'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'
>

const CODE_TO_KEY: Readonly<Record<string, string>> = {
  Space: 'Space',
  Tab: 'Tab',
  Enter: 'Enter',
  Escape: 'Escape',
  Backspace: 'Backspace',
  Delete: 'Delete',
  Insert: 'Insert',
  Home: 'Home',
  End: 'End',
  PageUp: 'PageUp',
  PageDown: 'PageDown',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  CapsLock: 'CapsLock',
  NumLock: 'NumLock',
  ScrollLock: 'ScrollLock',
  Pause: 'Pause',
  PrintScreen: 'PrintScreen',
  Backquote: 'Backquote',
  Backslash: 'Backslash',
  BracketLeft: 'BracketLeft',
  BracketRight: 'BracketRight',
  Comma: 'Comma',
  Equal: 'Equal',
  Minus: 'Minus',
  Period: 'Period',
  Quote: 'Quote',
  Semicolon: 'Semicolon',
  Slash: 'Slash',
  NumpadAdd: 'NumPadAdd',
  NumpadSubtract: 'NumPadSubtract',
  NumpadMultiply: 'NumPadMultiply',
  NumpadDivide: 'NumPadDivide',
  NumpadDecimal: 'NumPadDecimal',
  NumpadEnter: 'NumPadEnter',
  NumpadEqual: 'NumPadEqual'
}

function isModifierOnlyKey(key: string): boolean {
  return (
    key === 'Meta' ||
    key === 'Control' ||
    key === 'Alt' ||
    key === 'Shift' ||
    key === 'OS' ||
    key === 'Hyper' ||
    key === 'Super'
  )
}

function prefersMacModifiers(): boolean {
  if (typeof navigator === 'undefined') return false
  const platform = `${navigator.platform ?? ''} ${navigator.userAgent ?? ''}`.toLowerCase()
  return platform.includes('mac') || platform.includes('darwin')
}

function mapPrimaryKey(event: ShortcutKeyLike): string | null {
  const code = event.code
  if (/^Key[A-Z]$/u.test(code)) return code.slice(3)
  if (/^Digit[0-9]$/u.test(code)) return code.slice(5)
  if (/^F([1-9]|1\d|2[0-4])$/u.test(code)) return code
  if (/^Numpad[0-9]$/u.test(code)) return `Num${code.slice(6)}`

  const fromCode = CODE_TO_KEY[code]
  if (fromCode) return fromCode

  const key = event.key
  if (key.length === 1) {
    if (/[a-z]/iu.test(key)) return key.toUpperCase()
    if (/[0-9]/u.test(key)) return key
  }

  return null
}

/**
 * Convert a browser keyboard event into a Tauri global-shortcut string.
 * Returns null while the user is still holding only modifiers, or when the
 * combination cannot be expressed with the Tauri grammar we validate against.
 */
export function formatKeyboardEventToTauriShortcut(event: ShortcutKeyLike): string | null {
  if (isModifierOnlyKey(event.key)) return null

  const mainKey = mapPrimaryKey(event)
  if (!mainKey) return null

  const parts: string[] = []
  const mac = prefersMacModifiers()

  if (mac) {
    if (event.metaKey) parts.push('CommandOrControl')
    else if (event.ctrlKey) parts.push('Control')
  } else if (event.ctrlKey) {
    parts.push('CommandOrControl')
  } else if (event.metaKey) {
    parts.push('Super')
  }

  if (event.altKey) parts.push('Alt')
  if (event.shiftKey) parts.push('Shift')
  parts.push(mainKey)

  const shortcut = parts.join('+')
  return isValidTauriGlobalShortcut(shortcut) ? shortcut : null
}
