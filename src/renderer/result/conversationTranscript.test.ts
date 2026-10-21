import { describe, expect, it } from 'vitest'
import {
  appendUserTurn,
  beginAssistantTurn,
  isAskAction,
  parseTurnContent,
  patchStreamingAssistant,
  transcriptFromSnapshot,
  type TranscriptTurn
} from './conversationTranscript'

describe('conversationTranscript', () => {
  it('adds a pending assistant turn when a restored request is streaming', () => {
    expect(transcriptFromSnapshot('session-1', [
      { role: 'user', content: '什么是 title？' }
    ], true)).toEqual([
      {
        id: 'session-1:history:0',
        role: 'user',
        content: '什么是 title？',
        streaming: false
      },
      {
        id: 'session-1:history:assistant-pending',
        role: 'assistant',
        content: '',
        streaming: true
      }
    ])
  })

  it('appends user then assistant streaming patches', () => {
    let turns: TranscriptTurn[] = []
    turns = appendUserTurn(turns, '你好')
    turns = beginAssistantTurn(turns, 'a1')
    turns = patchStreamingAssistant(turns, '你', true)
    turns = patchStreamingAssistant(turns, '你好', false)
    expect(turns).toEqual([
      { id: expect.any(String), role: 'user', content: '你好', streaming: false },
      { id: 'a1', role: 'assistant', content: '你好', streaming: false }
    ])
  })

  it('ignores blank user questions', () => {
    expect(appendUserTurn([], '   ')).toEqual([])
    expect(appendUserTurn([], '   ', [])).toEqual([])
  })

  it('keeps attachment-only turns in the transcript and restored history', () => {
    const attachments = [{ name: 'photo.png', mime: 'image/png', kind: 'image', data: 'data123' }]
    const turns = appendUserTurn([], '   ', attachments)
    expect(turns).toEqual([
      { id: expect.any(String), role: 'user', content: '', attachments, streaming: false },
    ])
    const restored = transcriptFromSnapshot('session-1', turns, false)
    expect(restored[0]).toMatchObject({ role: 'user', content: '', attachments })
    expect(parseTurnContent(restored[0]!.content, restored[0]!.attachments)).toEqual({ text: '', attachments })
  })

  it('appends user turn with attachments', () => {
    const attachments = [
      { name: 'image.png', mime: 'image/png', kind: 'image', data: 'data123' },
    ]
    const turns = appendUserTurn([], '解释这张图', attachments)
    expect(turns).toEqual([
      {
        id: expect.any(String),
        role: 'user',
        content: '解释这张图',
        attachments,
        streaming: false,
      },
    ])
  })

  it('omits attachments property when attachments array is empty in appendUserTurn', () => {
    const turns = appendUserTurn([], '问题', [])
    expect(turns).toEqual([
      {
        id: expect.any(String),
        role: 'user',
        content: '问题',
        streaming: false,
      },
    ])
  })

  it('detects ask action kind', () => {
    expect(isAskAction('ask')).toBe(true)
    expect(isAskAction('translate')).toBe(false)
    expect(isAskAction(undefined)).toBe(false)
  })

  it('does not patch when there is no assistant turn', () => {
    const turns = appendUserTurn([], '问题')
    expect(patchStreamingAssistant(turns, '回复', true)).toBe(turns)
  })

  it('preserves attachment-like text without inventing attachments', () => {
    const parsed = parseTurnContent('这是一张图\n\n附件：demo.png、spec.pdf')
    expect(parsed.text).toBe('这是一张图\n\n附件：demo.png、spec.pdf')
    expect(parsed.attachments).toEqual([])
  })

  it('keeps explicit attachments separate from message text', () => {
    const parsed = parseTurnContent('这是一张图\n\n附件：demo.png', [
      { name: 'demo.png', mime: 'image/png', kind: 'image', data: 'base64data' }
    ])
    expect(parsed.text).toBe('这是一张图\n\n附件：demo.png')
    expect(parsed.attachments).toEqual([
      { name: 'demo.png', mime: 'image/png', kind: 'image', data: 'base64data' }
    ])
  })
})
