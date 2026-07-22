import { createElement } from 'react'
import { fireEvent, render, screen } from '@testing-library/react'

import { DEFAULT_ACTION_PROMPTS, type ActionDefinition } from '../../shared'
import { CustomActionDialog, promptAfterKindChange } from './CustomActionDialog'

describe('promptAfterKindChange', () => {
  it('uses the matching default prompt when a new action changes AI type', () => {
    expect(
      promptAfterKindChange('custom', 'translate', DEFAULT_ACTION_PROMPTS.custom)
    ).toBe(DEFAULT_ACTION_PROMPTS.translate)
    expect(
      promptAfterKindChange('translate', 'summary', DEFAULT_ACTION_PROMPTS.translate)
    ).toBe(DEFAULT_ACTION_PROMPTS.summary)
    expect(
      promptAfterKindChange('summary', 'explain', DEFAULT_ACTION_PROMPTS.summary)
    ).toBe(DEFAULT_ACTION_PROMPTS.explain)
  })

  it('does not overwrite a prompt the user has customized', () => {
    const customized = '请翻译成日语：{{text}}'
    expect(promptAfterKindChange('translate', 'summary', customized)).toBe(customized)
    expect(promptAfterKindChange('custom', 'copy', customized)).toBe(customized)
  })

  it('restores the current action type default without saving immediately', () => {
    const action: ActionDefinition = {
      id: 'translate-second',
      name: '第二个翻译',
      icon: 'languages',
      kind: 'translate',
      enabled: false,
      order: 8,
      prompt: '用户修改后的翻译提示词：{{text}}',
      providerId: 'provider-one',
      modelId: 'model-one',
      thinkingMode: 'off'
    }
    const onSave = vi.fn()
    render(createElement(CustomActionDialog, {
      action,
      providers: [{
        id: 'provider-one',
        name: '服务商一',
        baseUrl: 'https://example.com/v1',
        keyConfigured: true,
        models: [{ id: 'model-one', name: '模型一', thinkingLevels: [] }]
      }],
      onCancel: vi.fn(),
      onSave
    }))

    const textarea = document.getElementById('action-prompt') as HTMLTextAreaElement
    expect(textarea).toHaveValue(action.prompt)
    fireEvent.click(screen.getByRole('button', { name: '恢复默认提示词' }))
    expect(textarea).toHaveValue(DEFAULT_ACTION_PROMPTS.translate)
    expect(onSave).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: '保存修改' }))
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'translate',
      prompt: DEFAULT_ACTION_PROMPTS.translate
    }))
  })
})

describe('CustomActionDialog thinking mode', () => {
  it('shows thinking select when model supports levels and saves off by default', () => {
    const onSave = vi.fn()
    render(createElement(CustomActionDialog, {
      action: null,
      providers: [{
        id: 'provider-one',
        name: '服务商一',
        baseUrl: 'https://example.com/v1',
        keyConfigured: true,
        models: [{
          id: 'model-think',
          name: '思考模型',
          thinkingLevels: ['low', 'medium', 'high']
        }]
      }],
      onCancel: vi.fn(),
      onSave
    }))

    fireEvent.change(screen.getByLabelText('动作名称'), { target: { value: '深度思考' } })
    fireEvent.change(screen.getByLabelText('服务商'), { target: { value: 'provider-one' } })
    fireEvent.change(screen.getByLabelText('模型'), { target: { value: 'model-think' } })

    const thinkingSelect = screen.getByLabelText('思考强度')
    expect(thinkingSelect).toBeInTheDocument()
    expect(screen.getByRole('option', { name: '关闭思考（更快首字）' })).toBeInTheDocument()
    expect(thinkingSelect).toHaveValue('off')

    fireEvent.change(thinkingSelect, { target: { value: 'medium' } })
    fireEvent.click(screen.getByRole('button', { name: '添加动作' }))

    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'custom',
      modelId: 'model-think',
      thinkingMode: 'medium'
    }))
  })

  it('hides thinking select when model has no thinking levels', () => {
    render(createElement(CustomActionDialog, {
      action: null,
      providers: [{
        id: 'provider-one',
        name: '服务商一',
        baseUrl: 'https://example.com/v1',
        keyConfigured: true,
        models: [{ id: 'model-plain', name: '普通模型', thinkingLevels: [] }]
      }],
      onCancel: vi.fn(),
      onSave: vi.fn()
    }))

    fireEvent.change(screen.getByLabelText('服务商'), { target: { value: 'provider-one' } })
    fireEvent.change(screen.getByLabelText('模型'), { target: { value: 'model-plain' } })
    expect(screen.queryByLabelText('思考强度')).not.toBeInTheDocument()
  })
})
