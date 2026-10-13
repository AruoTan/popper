export function captureQuickAskShortcut(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>): string | null {
  if (!/^(Space|Key[A-Z]|Digit[0-9]|F(?:[1-9]|1[0-9]|2[0-4]))$/.test(event.code)) return null;
  const modifiers = [event.ctrlKey && 'Control', event.altKey && 'Alt', event.shiftKey && 'Shift', event.metaKey && 'Super'].filter(Boolean);
  return modifiers.length ? [...modifiers, event.code].join('+') : null;
}
export function displayQuickAskShortcut(value: string): string {
  return value.split('+').map((part) => part === 'Control' ? 'Ctrl' : part === 'Super' ? 'Win / ⌘' : part.replace(/^Key|^Digit/, '')).join(' + ');
}
