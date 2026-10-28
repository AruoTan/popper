import type { CSSProperties, JSX } from 'react';
import { Copy, Download, MessageSquare, Pin } from 'lucide-react';
import type { CaptureOutput } from '../../shared/capture';

export function CaptureOutputToolbar({ busy, onOutput, style, label }: {
  busy: boolean;
  onOutput: (output: CaptureOutput) => void;
  style?: CSSProperties;
  label: string;
}): JSX.Element {
  return <div className="capture-toolbar-surface capture-toolbar capture-output-toolbar" role="toolbar" aria-label={label} data-capture-ui style={style}>
    <button aria-label="保存 PNG" title="保存" disabled={busy} onClick={() => onOutput('file')}><Download size={16} />保存</button>
    <button aria-label="创建贴图" title="贴图" disabled={busy} onClick={() => onOutput('pin')}><Pin size={16} />贴图</button>
    <button aria-label="问 AI" title="问 AI" disabled={busy} onClick={() => onOutput('ask')}><MessageSquare size={16} />问 AI</button>
    <button aria-label="复制截图" title="复制" disabled={busy} onClick={() => onOutput('clipboard')}><Copy size={16} />复制</button>
  </div>;
}
