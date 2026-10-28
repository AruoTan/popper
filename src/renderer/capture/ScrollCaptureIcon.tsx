import type { JSX } from 'react';
import { Scan } from 'lucide-react';

export function ScrollCaptureIcon(): JSX.Element {
  return <Scan size={16} aria-hidden="true">
    <rect x="8" y="6" width="8" height="12" rx="4" />
    <path d="M12 6v4" />
  </Scan>;
}
