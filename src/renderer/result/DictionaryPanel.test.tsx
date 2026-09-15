import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { DictionarySnapshot, WindowTextLensApi } from '../../shared'
import { DictionaryPanel, useDictionarySession } from './DictionaryPanel'

const snapshot: DictionarySnapshot = {
  sessionId: 'session', revision: 2, queryGeneration: 1, query: 'account', mode: 'dictionary', status: 'found',
  entry: { word: 'account', ukPhone: 'əˈkaʊnt', usPhone: null, definitions: ['n. 账户', '<img src=x onerror=alert(1)>'],
    forms: [{ name: '复数', value: 'accounts' }], examples: [{ text: 'I have an account.', translation: '我有一个账户。' }] },
  suggestions: [{ word: 'account for', explanation: '解释' }], error: null, suggestionError: null
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => { resolve = r })
  return { promise, resolve }
}
const props = () => ({ snapshot, onAi: vi.fn().mockResolvedValue(true), onAsk: vi.fn(), onQuery: vi.fn() })
beforeEach(() => {
  window.textLens = {
    queryDictionary: vi.fn().mockResolvedValue('request'),
    suggestDictionary: vi.fn().mockResolvedValue([]), cancelDictionaryInput: vi.fn().mockResolvedValue(undefined),
    getEudicBooks: vi.fn().mockResolvedValue([{ id: '123456789012345678', name: '学习', language: 'en' }]),
    addEudicWord: vi.fn().mockResolvedValue(undefined)
  } as unknown as WindowTextLensApi
})
afterEach(() => vi.useRealTimers())

describe('dictionary panel', () => {
  it('renders structured data safely without AI or automatic audio', () => {
    const { container } = render(<DictionaryPanel {...props()} />)
    expect(screen.getByRole('heading', { name: 'account' })).toBeInTheDocument()
    expect(screen.getByText('n. 账户')).toBeInTheDocument()
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument()
    expect(container.querySelector('img')).toBeNull()
    expect(screen.queryByLabelText('播放美式发音')).not.toBeInTheDocument()
    expect(screen.getByText('I have an account.')).toBeInTheDocument()
  })
  it('queries the selected suggestion, not the first candidate automatically', async () => {
    const input = props(); render(<DictionaryPanel {...input} />)
    expect(window.textLens.queryDictionary).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'account for 解释' }))
    await waitFor(() => expect(window.textLens.queryDictionary).toHaveBeenCalledWith('session', 'account for'))
    expect(input.onQuery).toHaveBeenCalled()
  })
  it('debounces suggestions and ignores a late response for previous input', async () => {
    vi.useFakeTimers()
    const old = deferred<{ word: string; explanation: string }[]>()
    vi.mocked(window.textLens.suggestDictionary!).mockImplementation((_id, q) => q === 'acc' ? old.promise : Promise.resolve([{ word: 'cat', explanation: '猫' }]))
    render(<DictionaryPanel {...props()} />)
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'acc' } })
    await act(() => vi.advanceTimersByTimeAsync(249))
    expect(window.textLens.suggestDictionary).not.toHaveBeenCalled()
    await act(() => vi.advanceTimersByTimeAsync(1))
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'cat' } })
    await act(() => vi.advanceTimersByTimeAsync(250))
    await act(async () => old.resolve([{ word: 'account', explanation: '旧候选' }]))
    expect(screen.getByRole('button', { name: 'cat 猫' })).toBeInTheDocument()
    expect(screen.queryByText('旧候选')).not.toBeInTheDocument()
  })
  it('requires explicit book selection and prevents duplicate submission', async () => {
    const pending = deferred<void>(); vi.mocked(window.textLens.addEudicWord!).mockReturnValue(pending.promise)
    render(<DictionaryPanel {...props()} />)
    fireEvent.click(screen.getByRole('button', { name: '加入欧路生词本' }))
    const select = await screen.findByRole('combobox')
    expect(screen.getByRole('button', { name: '确认添加' })).toBeDisabled()
    fireEvent.change(select, { target: { value: '123456789012345678' } })
    fireEvent.click(screen.getByRole('button', { name: '确认添加' }))
    expect(screen.getByRole('button', { name: '正在添加…' })).toBeDisabled()
    expect(window.textLens.addEudicWord).toHaveBeenCalledExactlyOnceWith('session', 1, '123456789012345678')
    await act(async () => pending.resolve())
    expect(screen.getByRole('button', { name: '已添加' })).toBeDisabled()
  })
  it('keeps book selection after a failed add so the user can retry', async () => {
    vi.mocked(window.textLens.addEudicWord!).mockRejectedValue(new Error('授权已过期'))
    render(<DictionaryPanel {...props()} />)
    fireEvent.click(screen.getByRole('button', { name: '加入欧路生词本' }))
    fireEvent.change(await screen.findByRole('combobox'), { target: { value: '123456789012345678' } })
    fireEvent.click(screen.getByRole('button', { name: '确认添加' }))
    expect(await screen.findByText('授权已过期')).toBeInTheDocument()
    expect(screen.getByRole('combobox')).toHaveValue('123456789012345678')
    expect(screen.getByRole('button', { name: '确认添加' })).toBeEnabled()
  })
  it('does not show old add results after selecting a different word', async () => {
    const pending = deferred<void>(); vi.mocked(window.textLens.addEudicWord!).mockReturnValue(pending.promise)
    const p = props(); const { rerender } = render(<DictionaryPanel {...p} />)
    fireEvent.click(screen.getByRole('button', { name: '加入欧路生词本' }))
    fireEvent.change(await screen.findByRole('combobox'), { target: { value: '123456789012345678' } })
    fireEvent.click(screen.getByRole('button', { name: '确认添加' }))
    rerender(<DictionaryPanel {...p} snapshot={{ ...snapshot, query: 'cat', queryGeneration: 2, entry: { ...snapshot.entry!, word: 'cat' } }} />)
    await act(async () => pending.resolve())
    expect(screen.queryByText(/已加入所选生词本/)).not.toBeInTheDocument()
  })
  it('keeps lookup errors separate from a missing entry', () => {
    render(<DictionaryPanel {...props()} snapshot={{ ...snapshot, status: 'error', entry: null, error: '网络请求超时' }} />)
    expect(screen.getByRole('alert')).toHaveTextContent('网络请求超时')
    expect(screen.getByRole('button', { name: '重试查词' })).toBeInTheDocument()
    expect(screen.queryByText(/没有找到词典释义/)).not.toBeInTheDocument()
  })
  it('ignores old snapshot hydration and events for other windows', async () => {
    const initial = deferred<DictionarySnapshot | null>(); let listener!: (s: DictionarySnapshot) => void
    window.textLens.getDictionaryState = vi.fn().mockReturnValue(initial.promise)
    window.textLens.onDictionaryChanged = (fn) => { listener = fn; return () => {} }
    const { result } = renderHook(() => useDictionarySession('session'))
    act(() => listener({ ...snapshot, revision: 4 }))
    await act(async () => initial.resolve(snapshot))
    act(() => listener({ ...snapshot, sessionId: 'another', revision: 6 }))
    expect(result.current?.revision).toBe(4)
  })
})
