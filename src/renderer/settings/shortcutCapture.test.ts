import { formatKeyboardEventToTauriShortcut, SUGGESTED_CAPTURE_SHORTCUT } from './shortcutCapture'

function event(partial: Partial<KeyboardEvent> & Pick<KeyboardEvent, 'key' | 'code'>): KeyboardEvent {
  return {
    key: partial.key,
    code: partial.code,
    metaKey: partial.metaKey ?? false,
    ctrlKey: partial.ctrlKey ?? false,
    altKey: partial.altKey ?? false,
    shiftKey: partial.shiftKey ?? false
  } as KeyboardEvent
}

describe('formatKeyboardEventToTauriShortcut', () => {
  const originalUserAgent = navigator.userAgent
  const originalPlatform = navigator.platform

  afterEach(() => {
    Object.defineProperty(navigator, 'userAgent', { configurable: true, value: originalUserAgent })
    Object.defineProperty(navigator, 'platform', { configurable: true, value: originalPlatform })
  })

  it('ignores bare modifier presses', () => {
    expect(
      formatKeyboardEventToTauriShortcut(
        event({ key: 'Meta', code: 'MetaLeft', metaKey: true })
      )
    ).toBeNull()
  })

  it('maps macOS Command chords to CommandOrControl', () => {
    Object.defineProperty(navigator, 'platform', { configurable: true, value: 'MacIntel' })
    Object.defineProperty(navigator, 'userAgent', {
      configurable: true,
      value: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'
    })

    expect(
      formatKeyboardEventToTauriShortcut(
        event({ key: 's', code: 'KeyS', metaKey: true, shiftKey: true })
      )
    ).toBe('CommandOrControl+Shift+S')
  })

  it('maps Windows Control chords to CommandOrControl', () => {
    Object.defineProperty(navigator, 'platform', { configurable: true, value: 'Win32' })
    Object.defineProperty(navigator, 'userAgent', {
      configurable: true,
      value: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'
    })

    expect(
      formatKeyboardEventToTauriShortcut(
        event({ key: 's', code: 'KeyS', ctrlKey: true, shiftKey: true })
      )
    ).toBe('CommandOrControl+Shift+S')
  })

  it('keeps every pressed primary modifier in multi-modifier chords', () => {
    Object.defineProperty(navigator, 'platform', { configurable: true, value: 'MacIntel' })
    Object.defineProperty(navigator, 'userAgent', {
      configurable: true,
      value: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'
    })
    expect(
      formatKeyboardEventToTauriShortcut(
        event({ key: 'k', code: 'KeyK', metaKey: true, ctrlKey: true })
      )
    ).toBe('CommandOrControl+Control+K')

    Object.defineProperty(navigator, 'platform', { configurable: true, value: 'Win32' })
    Object.defineProperty(navigator, 'userAgent', {
      configurable: true,
      value: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'
    })
    expect(
      formatKeyboardEventToTauriShortcut(
        event({ key: 'k', code: 'KeyK', metaKey: true, ctrlKey: true })
      )
    ).toBe('CommandOrControl+Super+K')
  })

  it('maps arrow and function keys', () => {
    Object.defineProperty(navigator, 'platform', { configurable: true, value: 'MacIntel' })
    expect(
      formatKeyboardEventToTauriShortcut(
        event({ key: 'ArrowUp', code: 'ArrowUp', metaKey: true })
      )
    ).toBe('CommandOrControl+Up')
    expect(
      formatKeyboardEventToTauriShortcut(event({ key: 'F5', code: 'F5', altKey: true }))
    ).toBe('Alt+F5')
  })

  it('exposes the suggested capture shortcut used by the settings UI', () => {
    expect(SUGGESTED_CAPTURE_SHORTCUT).toBe('CommandOrControl+Shift+S')
  })
})
