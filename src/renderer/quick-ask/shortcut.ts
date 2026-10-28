const STANDALONE_KEYS = /^(PrintScreen|F(?:[1-9]|1[0-9]|2[0-4])|Pause|ScrollLock|Insert)$/;
const ALL_KEYS = /^(PrintScreen|Pause|ScrollLock|Insert|Delete|Home|End|PageUp|PageDown|ArrowUp|ArrowDown|ArrowLeft|ArrowRight|Space|Tab|Enter|Backspace|CapsLock|NumLock|Escape|Key[A-Z]|Digit[0-9]|F(?:[1-9]|1[0-9]|2[0-4])|Numpad[0-9]|Numpad(?:Add|Subtract|Multiply|Divide|Decimal|Enter|Equal)|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Comma|Period|Slash)$/;

export const SHORTCUT_DISPLAY_NAMES: Record<string, string> = {
  Control: 'Ctrl',
  Super: 'Win / ⌘',
  PrintScreen: 'PrtSc',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  Backquote: '`',
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
};

export function captureQuickAskShortcut(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>): string | null {
  if (!ALL_KEYS.test(event.code)) return null;
  const modifiers = [event.ctrlKey && 'Control', event.metaKey && 'Super', event.altKey && 'Alt', event.shiftKey && 'Shift'].filter(Boolean);
  if (modifiers.length === 0 && !STANDALONE_KEYS.test(event.code)) return null;
  return modifiers.length ? [...modifiers, event.code].join('+') : event.code;
}

export function displayQuickAskShortcut(value: string): string {
  return value
    .split('+')
    .map((part) => SHORTCUT_DISPLAY_NAMES[part] ?? part.replace(/^Key|^Digit/, ''))
    .join(' + ');
}
