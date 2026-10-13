import { CircleAlert, FileText, LoaderCircle, Paperclip, SendHorizontal, Settings2, X } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState, type JSX } from 'react';
import { PopperIcon } from '../components/PopperIcon';
import { getErrorMessage } from '../lib/errors';
import { ATTACHMENT_ACCEPT, attachmentSize, readAttachment, validateAttachmentList, type SelectedAttachment } from './attachments';

export function QuickAskApp(): JSX.Element {
  const input = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const container = useRef<HTMLElement>(null);
  const busyRef = useRef(false);
  const mounted = useRef(true);
  const [question, setQuestion] = useState('');
  const [attachments, setAttachments] = useState<SelectedAttachment[]>([]);
  const [reading, setReading] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState('');
  const [dragging, setDragging] = useState(false);
  const busy = reading || sending;

  useEffect(() => {
    mounted.current = true;
    const focus = (): void => { requestAnimationFrame(() => input.current?.focus()); };
    const unsubscribe = window._popper_.onQuickAskOpened?.(focus);
    void window._popper_.quickAskReady?.().then(focus).catch((error: unknown) => {
      if (mounted.current) setError(getErrorMessage(error, '无法打开快捷提问'));
    });
    focus();
    return () => { mounted.current = false; unsubscribe?.(); };
  }, []);

  useLayoutEffect(() => {
    const element = container.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    const report = (): void => {
      // Measure natural content as well as the visible box so a small display
      // can scroll while adding attachments still grows a compact composer.
      void window._popper_.reportQuickAskSize?.(Math.ceil(element.scrollHeight) + 26).catch(() => {});
    };
    const observer = new ResizeObserver(report);
    observer.observe(element);
    report();
    return () => observer.disconnect();
  }, [attachments, error]);

  const addFiles = async (files: File[]): Promise<void> => {
    if (!files.length || busyRef.current) return;
    busyRef.current = true;
    setReading(true);
    setError('');
    try {
      if (files.length + attachments.length > 6) throw new Error('最多添加 6 个附件');
      const next = [...attachments, ...await Promise.all(files.map(readAttachment))];
      validateAttachmentList(next);
      if (mounted.current) setAttachments(next);
    } catch (error: unknown) {
      if (mounted.current) setError(getErrorMessage(error, '无法添加附件'));
    } finally {
      busyRef.current = false;
      if (mounted.current) { setReading(false); input.current?.focus(); }
    }
  };

  const submit = async (): Promise<void> => {
    if (!question.trim() || busyRef.current) return;
    busyRef.current = true;
    setSending(true);
    setError('');
    try {
      if (Array.from(question.trim()).length > 20_000) throw new Error('问题最多 20,000 个字符');
      if (!window._popper_.submitQuickAsk) throw new Error('请重启 Popper 以启用快捷提问');
      const result = await window._popper_.submitQuickAsk(question.trim(), attachments.map(({ payload }) => payload));
      if (!result.accepted) throw new Error(result.message);
      if (mounted.current) { setQuestion(''); setAttachments([]); }
    } catch (error: unknown) {
      if (mounted.current) { setError(getErrorMessage(error, '发送失败')); input.current?.focus(); }
    } finally {
      busyRef.current = false;
      if (mounted.current) setSending(false);
    }
  };

  const close = (): void => { if (!busyRef.current) void window._popper_.closeQuickAsk?.().catch((error: unknown) => setError(getErrorMessage(error, '无法关闭窗口'))); };

  return (
    <main ref={container} className={`quick-ask ${dragging ? 'quick-ask--dragging' : ''}`}
      onKeyDown={(event) => { if (event.key === 'Escape') { event.preventDefault(); close(); } }}
      onDragOver={(event) => { event.preventDefault(); if (!busy) setDragging(true); }}
      onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(event) => { event.preventDefault(); setDragging(false); void addFiles(Array.from(event.dataTransfer.files)); }}>
      <header className="quick-ask__header" data-tauri-drag-region>
        <div className="quick-ask__identity" data-tauri-drag-region><PopperIcon size={25} /><span>问 AI</span><span className="quick-ask__badge">快捷提问</span></div>
        <div className="quick-ask__window-actions">
          <button type="button" className="quick-ask__icon-button" aria-label="打开设置" disabled={busy} onClick={() => void window._popper_.openSettings?.({ focus: 'providers' }).catch((error: unknown) => setError(getErrorMessage(error, '无法打开设置')))}><Settings2 size={15} /></button>
          <button type="button" className="quick-ask__icon-button" aria-label="关闭快捷提问" disabled={busy} onClick={close}><X size={16} /></button>
        </div>
      </header>
      {attachments.length > 0 && <section className="quick-ask__attachments" aria-label="附件预览">
        <div className="quick-ask__attachment-heading"><span>附件</span><span>{attachments.length} / 6</span></div>
        <div className="quick-ask__attachment-list">{attachments.map(({ id, size, payload }) => <article className="quick-ask__attachment" key={id}>
          {payload.kind === 'image' ? <img src={`data:${payload.mime};base64,${payload.data}`} alt={payload.name} /> : <span className="quick-ask__file-icon"><FileText size={20} /></span>}
          <div className="quick-ask__file-info"><strong title={payload.name}>{payload.name}</strong><span>{payload.kind === 'image' ? '图片' : payload.kind === 'file' ? (payload.name.includes('.') ? payload.name.split('.').at(-1)?.toUpperCase() : '文件') : '文本'} · {attachmentSize(size)}</span></div>
          <button className="quick-ask__icon-button" type="button" aria-label={`移除 ${payload.name}`} disabled={busy} onClick={() => setAttachments((current) => current.filter((file) => file.id !== id))}><X size={14} /></button>
        </article>)}</div>
        {attachments.some(({ payload }) => payload.kind !== "text") && <p className="quick-ask__attachment-note">图片需要模型支持；文档解析取决于接入方式和可用工具。</p>}
      </section>}
      <textarea ref={input} className="quick-ask__input" aria-label="快捷提问内容" placeholder="有什么想问的？" value={question} disabled={sending}
        onChange={(event) => setQuestion(event.target.value)}
        onPaste={(event) => { if (event.clipboardData.files.length) { event.preventDefault(); void addFiles(Array.from(event.clipboardData.files)); } }}
        onKeyDown={(event) => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void submit(); } }} />
      {error && <div className="quick-ask__error" role="alert"><CircleAlert size={14} /><span>{error}</span></div>}
      <footer className="quick-ask__footer">
        <input ref={picker} type="file" multiple accept={ATTACHMENT_ACCEPT} aria-label="选择附件" hidden onChange={(event) => { const files = Array.from(event.target.files ?? []); event.target.value = ''; void addFiles(files); }} />
        <button type="button" className="quick-ask__attach" disabled={busy} onClick={() => picker.current?.click()} title="图片、文档或其他文件 · 每个最多 8 MB，合计最多 20 MB">{reading ? <LoaderCircle size={16} className="quick-ask__spin" /> : <Paperclip size={16} />}<span>{reading ? '读取中' : '添加附件'}</span></button>
        <span className="quick-ask__hint">Enter 发送 · Shift + Enter 换行</span>
        <button type="button" className="quick-ask__send" aria-label="发送问题" disabled={busy || !question.trim()} onClick={() => void submit()}>{sending ? <LoaderCircle className="quick-ask__spin" size={17} /> : <SendHorizontal size={17} />}</button>
      </footer>
      {dragging && <div className="quick-ask__drop-hint">松开以添加附件</div>}
    </main>
  );
}
