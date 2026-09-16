import { describe, expect, it } from 'vitest'
import {
  appendUserTurn,
  beginAssistantTurn,
  isAskAction,
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
})
