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
    expect(promptAfterKindChange('custom', 'refine', customized)).toBe(customized)
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
        enabled: true,
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

describe('CustomActionDialog model + thinking', () => {
  function choose(label: string, option: string): void {
    fireEvent.click(screen.getByRole('combobox', { name: label }))
    fireEvent.click(screen.getByRole('option', { name: option }))
  }

  it('closes the open dropdown before closing the dialog with Escape', () => {
    const onCancel = vi.fn()
    render(createElement(CustomActionDialog, { action: null, providers: [], onCancel, onSave: vi.fn() }))
    const trigger = screen.getByRole('combobox', { name: '动作类型' })
    fireEvent.click(trigger)
    fireEvent.keyDown(trigger, { key: 'Escape' })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(onCancel).not.toHaveBeenCalled()
    fireEvent.keyDown(trigger, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledOnce()
  })

  it('offers AI action types without retired local actions', () => {
    render(createElement(CustomActionDialog, { action: null, providers: [], onCancel: vi.fn(), onSave: vi.fn() }))
    fireEvent.click(screen.getByRole('combobox', { name: '动作类型' }))
    expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual(['翻译', '总结', '解释', '润色', '自定义 AI'])
    expect(screen.queryByRole('combobox', { name: '默认搜索引擎' })).not.toBeInTheDocument()
  })

  it('uses a unified provider+model list and saves thinking mode', () => {
    const onSave = vi.fn()
    render(createElement(CustomActionDialog, {
      action: null,
      providers: [{
        id: 'provider-one',
        name: '服务商一',
        enabled: true,
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
    choose('模型', '思考模型')

    const thinkingSelect = screen.getByLabelText('思考强度')
    expect(thinkingSelect).toBeInTheDocument()
    expect(thinkingSelect).toHaveTextContent('关闭思考（更快首字）')

    choose('思考强度', '中')
    fireEvent.click(screen.getByRole('button', { name: '添加动作' }))

    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'custom',
      providerId: 'provider-one',
      modelId: 'model-think',
      thinkingMode: 'medium'
    }))
  })

  it('hides disabled providers and infers thinking from model id', () => {
    render(createElement(CustomActionDialog, {
      action: null,
      providers: [
        {
          id: 'provider-off',
          name: '已关闭',
          enabled: false,
          baseUrl: 'https://example.com/v1',
          keyConfigured: true,
          models: [{ id: 'hidden-model', name: '隐藏模型', thinkingLevels: [] }]
        },
        {
          id: 'provider-on',
          name: '启用',
          enabled: true,
          baseUrl: 'https://example.com/v1',
          keyConfigured: true,
          models: [{ id: 'deepseek-r1', name: 'DeepSeek R1', thinkingLevels: [] }]
        }
      ],
      onCancel: vi.fn(),
      onSave: vi.fn()
    }))

    fireEvent.click(screen.getByRole('combobox', { name: '模型' }))
    expect(screen.queryByRole('option', { name: '隐藏模型' })).not.toBeInTheDocument()
    expect(screen.getByText('启用', { selector: '.styled-select__group' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('option', { name: 'DeepSeek R1' }))
    expect(screen.getByLabelText('思考强度')).toBeInTheDocument()
  })

  it('hides thinking select when model has no thinking levels', () => {
    render(createElement(CustomActionDialog, {
      action: null,
      providers: [{
        id: 'provider-one',
        name: '服务商一',
        enabled: true,
        baseUrl: 'https://example.com/v1',
        keyConfigured: true,
        models: [{ id: 'model-plain', name: '普通模型', thinkingLevels: [] }]
      }],
      onCancel: vi.fn(),
      onSave: vi.fn()
    }))

    choose('模型', '普通模型')
    expect(screen.queryByLabelText('思考强度')).not.toBeInTheDocument()
  })
})
