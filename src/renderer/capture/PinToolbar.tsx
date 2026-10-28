import type { JSX } from 'react';
import { Copy, Download, Redo2, Trash2, Undo2, X } from 'lucide-react';
import type { PinToolbarAction, PinToolbarState } from '../../shared/capture';
import { ANNOTATION_TOOLS } from './annotationTools';
import { clamp } from './geometry';

export function PinToolbar({ state, onAction }: { state: PinToolbarState; onAction: (action: PinToolbarAction) => void }): JSX.Element {
  return <div className="capture-toolbar-surface capture-pin-toolbar" role="toolbar" aria-label="贴图工具条" data-pin-ui>
    {ANNOTATION_TOOLS.map(([value, Icon, title]) => <button key={value} title={title} aria-label={title} aria-pressed={state.tool === value} disabled={!state.enabled} onClick={() => onAction({ action: 'tool', tool: value })}><Icon size={16} /></button>)}
    <input type="color" aria-label="标注颜色" value={state.color} disabled={!state.enabled} onChange={(event) => onAction({ action: 'color', color: event.target.value })} />
    <input type="number" aria-label="线宽" min={1} max={32} value={state.lineWidth} disabled={!state.enabled} onChange={(event) => onAction({ action: 'width', width: clamp(Number(event.target.value) || 1, 1, 32) })} />
    <button aria-label="撤销" title="撤销" disabled={!state.enabled || !state.canUndo} onClick={() => onAction({ action: 'undo' })}><Undo2 size={16} /></button>
    <button aria-label="重做" title="重做" disabled={!state.enabled || !state.canRedo} onClick={() => onAction({ action: 'redo' })}><Redo2 size={16} /></button>
    <button aria-label="删除标注" title="删除标注" disabled={!state.enabled || !state.canDelete} onClick={() => onAction({ action: 'delete' })}><Trash2 size={16} /></button>
    <button aria-label="复制图像" title="复制图像" disabled={state.busy} onClick={() => onAction({ action: 'copy' })}><Copy size={16} /></button>
    <button aria-label="另存图像" title="另存图像" disabled={state.busy} onClick={() => onAction({ action: 'save' })}><Download size={16} /></button>
    <button aria-label="隐藏工具条" title="隐藏工具条" disabled={state.busy} onClick={() => onAction({ action: 'hide' })}><X size={16} /></button>
  </div>;
}
