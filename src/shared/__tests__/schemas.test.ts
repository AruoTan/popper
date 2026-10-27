import { describe, expect, it } from 'vitest'

import {
  AI_OUTPUT_LIMIT,
  DEFAULT_ACTIONS,
  DEFAULT_ACTION_PROMPTS,
  DEFAULT_APP_SETTINGS,
  DEFAULT_PROVIDER_ID,
  DEFAULT_PUBLIC_SETTINGS,
  MAX_ENABLED_ACTIONS,
  SETTINGS_VERSION,
  TEXT_PLACEHOLDER,
  actionDefinitionSchema,
  actionStreamEventSchema,
  actionsSchema,
  appSettingsSchema,
  customActionSchema,
  providerModelSchema,
  publicSettingsSchema,
  resultRendererMarkerSchema,
  resultSessionSnapshotSchema,
  settingsUpdateSchema,
  toPublicSettings
} from '..'

describe('settings schemas and defaults', () => {
  it('preserves current settings and rejects other formats without migration', () => {
    const settings = { ...structuredClone(DEFAULT_PUBLIC_SETTINGS) }
    settings.trayEnabled = false
    settings.quickAskShortcut = 'Alt+KeyK'
    settings.actions[0]!.prompt = '自定义：{{text}}'
    expect(publicSettingsSchema.parse(settings)).toEqual(settings)
    for (const version of [1, 2, 11, 16, SETTINGS_VERSION + 1]) {
      expect(publicSettingsSchema.safeParse({ ...settings, version }).success).toBe(false)
    }
    for (const field of ['trayEnabled', 'quickAskShortcut', 'defaultAi'] as const) {
      const incomplete = structuredClone(settings) as Record<string, unknown>
      delete incomplete[field]
      expect(publicSettingsSchema.safeParse(incomplete).success).toBe(false)
    }
    expect(publicSettingsSchema.safeParse({ ...settings, selectionCapture: {} }).success).toBe(false)
    expect(publicSettingsSchema.safeParse({ ...settings, providers: settings.providers.map(({ kind: _kind, ...provider }) => provider) }).success).toBe(false)
    expect(settingsUpdateSchema.parse({ quickAskShortcut: 'Alt+KeyK', trayEnabled: false })).toEqual({ quickAskShortcut: 'Alt+KeyK', trayEnabled: false })
  })
  it('round-trips DSH settings and rejects ambiguous model routes and host workspace paths', () => {
    const settings = publicSettingsSchema.parse(structuredClone(DEFAULT_PUBLIC_SETTINGS));
    settings.providers[0] = {
      ...settings.providers[0]!, kind: 'dsh', baseUrl: 'http://localhost:3080', dshWorkspace: '/workspace',
      models: [{ id: 'provider/org/model', name: 'Model', thinkingLevels: [] }]
    };
    expect(publicSettingsSchema.parse(settings).providers[0]).toEqual(settings.providers[0]);
    expect(publicSettingsSchema.safeParse(settings).success).toBe(true);
    settings.providers[0]!.models[0]!.id = 'model';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(false);
    settings.providers[0]!.models[0]!.id = 'provider/model';
    settings.providers[0]!.dshWorkspace = 'C:\\project';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(false);
    expect(publicSettingsSchema.safeParse(DEFAULT_PUBLIC_SETTINGS).success).toBe(true);
  });

  it('provides v18 defaults with only AI actions', () => {
    expect(appSettingsSchema.parse(DEFAULT_APP_SETTINGS)).toEqual(DEFAULT_APP_SETTINGS)
    expect(DEFAULT_APP_SETTINGS.version).toBe(18)
    expect(DEFAULT_APP_SETTINGS.providers[0]).toMatchObject({
      id: DEFAULT_PROVIDER_ID,
      enabled: true,
      baseUrl: 'https://api.openai.com/v1',
      apiKey: '',
      models: []
    })
    expect(DEFAULT_APP_SETTINGS.translate).toEqual({
      dictionaryEnabled: true,
      primaryLanguage: 'zh-CN',
      alternateLanguage: 'en-US'
    })
    expect(DEFAULT_APP_SETTINGS.providers).toHaveLength(1)
    expect(DEFAULT_APP_SETTINGS.result).toMatchObject({
      followCursor: true,
      rememberSize: true,
      defaultPinned: false,
      dismissMode: 'blur',
      fontSize: 14,
      lastSize: null
    })
    expect(DEFAULT_ACTIONS.map((action) => action.kind)).toEqual([
      'translate',
      'explain',
      'summary',
      'refine'
    ])
    expect(DEFAULT_ACTIONS.some((action) => action.kind === 'ask')).toBe(false)
    for (const action of DEFAULT_ACTIONS) expect(action.prompt).toContain(TEXT_PLACEHOLDER)
  })

  it('rejects retired capture configuration in settings updates', () => {
    expect(settingsUpdateSchema.safeParse({ trigger: { mode: 'selected' } }).success).toBe(false)
    expect(settingsUpdateSchema.safeParse({ captureShortcut: 'Ctrl+F12' }).success).toBe(false)
    expect(settingsUpdateSchema.safeParse({ selectionCapture: { defaultStrategy: 'auto' } }).success).toBe(false)
  })

  it('rejects removed behavior options in settings updates', () => {
    for (const option of [{ enabled: false }, { toolbar: { displayMode: 'icon-only' } }, { application: { closeBehavior: 'quit' } }]) {
      expect(settingsUpdateSchema.safeParse(option).success).toBe(false)
    }
  })

  it('accepts ask AI actions and rejects quote local actions', () => {
    expect(() => actionDefinitionSchema.parse({
      id: 'ask-ai',
      name: '问AI',
      icon: 'message-circle-question',
      kind: 'ask',
      enabled: true,
      order: 0,
      prompt: `上下文：${TEXT_PLACEHOLDER}`,
      providerId: 'openai-compatible',
      modelId: 'gpt',
      thinkingMode: 'off'
    })).not.toThrow()

    expect(() => actionDefinitionSchema.parse({
      id: 'quote',
      name: '引用',
      icon: 'quote',
      kind: 'quote',
      enabled: true,
      order: 0
    })).toThrow()
  })

  it('never includes provider API keys in public settings', () => {
    const settings = appSettingsSchema.parse({
      ...DEFAULT_APP_SETTINGS,
      providers: DEFAULT_APP_SETTINGS.providers.map((provider, index) => ({
        ...provider,
        apiKey: index === 0 ? 'super-secret' : ''
      }))
    })
    const publicSettings = toPublicSettings(settings)

    expect(publicSettings.providers[0]?.keyConfigured).toBe(true)
    expect(publicSettings.providers[0]).not.toHaveProperty('apiKey')
    expect(publicSettingsSchema.parse(DEFAULT_PUBLIC_SETTINGS)).toEqual(DEFAULT_PUBLIC_SETTINGS)
    expect(
      publicSettingsSchema.safeParse({
        ...publicSettings,
        providers: [{ ...publicSettings.providers[0], apiKey: 'must-be-rejected' }]
      }).success
    ).toBe(false)
  })

  it('requires every AI prompt to contain the text placeholder', () => {
    const base = {
      id: 'custom-one',
      name: '自定义',
      icon: 'sparkles',
      kind: 'custom' as const,
      enabled: false,
      order: 10,
      providerId: DEFAULT_PROVIDER_ID,
      modelId: ''
    }
    expect(customActionSchema.safeParse({ ...base, prompt: '解释这段内容' }).success).toBe(false)
    expect(
      customActionSchema.safeParse({ ...base, prompt: `解释：${TEXT_PLACEHOLDER}` }).success
    ).toBe(true)
  })

  it('defaults thinkingLevels to [] and thinkingMode to off', () => {
    const model = providerModelSchema.parse({ id: 'm', name: 'M' })
    expect(model.thinkingLevels).toEqual([])

    const action = actionDefinitionSchema.parse({
      id: 'translate',
      name: '翻译',
      icon: 'languages',
      kind: 'translate',
      enabled: true,
      order: 0,
      prompt: 'x {{text}}',
      providerId: 'p',
      modelId: 'm'
    })
    expect(action).toMatchObject({ thinkingMode: 'off' })

  })

  it('accepts thinking levels on models and modes on AI actions', () => {
    const model = providerModelSchema.parse({
      id: 'o3-mini',
      name: 'o3-mini',
      thinkingLevels: ['low', 'medium', 'high']
    })
    expect(model.thinkingLevels).toEqual(['low', 'medium', 'high'])

    const action = actionDefinitionSchema.parse({
      id: 'translate',
      name: '翻译',
      icon: 'languages',
      kind: 'translate',
      enabled: true,
      order: 0,
      prompt: 'x {{text}}',
      providerId: 'p',
      modelId: 'o3-mini',
      thinkingMode: 'medium'
    })
    expect(action).toMatchObject({ thinkingMode: 'medium' })

    expect(
      providerModelSchema.safeParse({
        id: 'bad',
        name: 'Bad',
        thinkingLevels: ['ultra']
      }).success
    ).toBe(false)
    expect(
      actionDefinitionSchema.safeParse({
        id: 'translate',
        name: '翻译',
        icon: 'languages',
        kind: 'translate',
        enabled: true,
        order: 0,
        prompt: 'x {{text}}',
        providerId: 'p',
        modelId: 'm',
        thinkingMode: 'turbo'
      }).success
    ).toBe(false)
  })

  it('accepts models with and without thinking capability evidence', () => {
    const basic = providerModelSchema.parse({ id: 'basic', name: 'Basic', thinkingLevels: [] })
    expect(basic.thinkingCapability).toBeUndefined()

    const explicit = providerModelSchema.parse({
      id: 'o3-mini',
      name: 'o3-mini',
      thinkingLevels: ['low', 'medium'],
      thinkingCapability: {
        source: 'explicit',
        dialect: 'reasoningEffort',
        supportsOff: false
      }
    })
    expect(explicit.thinkingCapability).toEqual({
      source: 'explicit',
      dialect: 'reasoningEffort',
      supportsOff: false
    })

    expect(
      providerModelSchema.safeParse({
        id: 'bad-capability',
        name: 'Bad capability',
        thinkingLevels: [],
        thinkingCapability: {
          source: 'explicit',
          dialect: 'reasoningEffort',
          supportsOff: true,
          extra: true
        }
      }).success
    ).toBe(false)
  })

  it('allows duplicate kinds but rejects duplicate action IDs', () => {
    const first = DEFAULT_ACTIONS.find((action) => action.kind === 'translate')!
    const second = { ...first, id: 'translate-second', name: '第二个翻译', order: 20, enabled: false }
    expect(actionsSchema.safeParse([...DEFAULT_ACTIONS, second]).success).toBe(true)
    expect(actionsSchema.safeParse([first, first]).success).toBe(false)
  })

  it('requires one through eight enabled actions', () => {
    const enabled = Array.from({ length: MAX_ENABLED_ACTIONS }, (_, index) => ({
      ...DEFAULT_ACTIONS[0]!,
      id: `translate-${index}`,
      name: `动作 ${index}`,
      icon: 'clipboard-copy',
      kind: 'translate' as const,
      enabled: true,
      order: index
    }))
    expect(actionsSchema.safeParse(enabled).success).toBe(true)
    expect(actionsSchema.safeParse([...enabled, { ...enabled[0]!, id: 'overflow' }]).success).toBe(false)
    expect(actionsSchema.safeParse(enabled.map((action) => ({ ...action, enabled: false }))).success).toBe(false)
  })

  it('does not allow API keys through the ordinary settings update channel', () => {
    expect(
      settingsUpdateSchema.safeParse({
        providers: [{
          id: 'provider',
          name: 'Provider',
          baseUrl: 'https://example.com/v1',
          apiKey: 'secret',
          models: []
        }]
      }).success
    ).toBe(false)
  })

  it('validates the supported result text size range', () => {
    expect(publicSettingsSchema.safeParse({
      ...DEFAULT_PUBLIC_SETTINGS,
      result: { ...DEFAULT_PUBLIC_SETTINGS.result, fontSize: 12 }
    }).success).toBe(true)
    expect(publicSettingsSchema.safeParse({
      ...DEFAULT_PUBLIC_SETTINGS,
      result: { ...DEFAULT_PUBLIC_SETTINGS.result, fontSize: 24 }
    }).success).toBe(true)
    expect(publicSettingsSchema.safeParse({
      ...DEFAULT_PUBLIC_SETTINGS,
      result: { ...DEFAULT_PUBLIC_SETTINGS.result, fontSize: 25 }
    }).success).toBe(false)
  })
})

describe('action stream event schemas', () => {
  it('allows empty source only for an explicit quick ask session', () => {
    const snapshot = {
      sessionId: 'session-quick', sessionGeneration: 1, requestId: 'request-quick', requestGeneration: 1,
      actionId: 'ask-ai', quickAsk: true,
      selection: { text: '', sourceApp: { name: 'Popper', bundleId: null }, anchor: { kind: 'cursor', x: 0, y: 0 } },
      status: 'completed', content: '', contentScalarCount: 0, lastSequence: 0, lastContentSequence: 0, handshakeGeneration: 1,
      errorMessage: '', retryable: false, pinned: false
    };
    expect(resultSessionSnapshotSchema.safeParse(snapshot).success).toBe(true);
    expect(resultSessionSnapshotSchema.safeParse({ ...snapshot, quickAsk: false }).success).toBe(false);
    expect(resultSessionSnapshotSchema.safeParse({ ...snapshot, actionId: 'translate' }).success).toBe(false);
  });

  const orderedBase = {
    sessionId: 'session-1',
    sessionGeneration: 7,
    requestId: 'request-2',
    requestGeneration: 2,
    sequence: 11,
    actionId: 'translate'
  } as const

  it('accepts the exact ordered event variants used by the native stream', () => {
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'delta',
        delta: '😀'
      }).success
    ).toBe(true)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'thinkingDelta',
        delta: 'reason'
      }).success
    ).toBe(true)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'completed',
        lastContentSequence: 10,
        contentScalarCount: 1
      }).success
    ).toBe(true)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'resyncRequired',
        snapshotLastSequence: 15
      }).success
    ).toBe(true)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'notice',
        code: 'THINKING_CONTROL_FALLBACK',
        message: '服务商不支持当前的“关闭思考”控制，本次已按服务商默认设置继续生成。'
      }).success
    ).toBe(true)
  })

  it('rejects unordered, unsafe, fractional, expanded, and full-content completion payloads', () => {
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        sequence: 0,
        type: 'started'
      }).success
    ).toBe(false)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        requestGeneration: -1,
        type: 'started'
      }).success
    ).toBe(false)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        sessionGeneration: 1.5,
        type: 'started'
      }).success
    ).toBe(false)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        sequence: Number.MAX_SAFE_INTEGER + 1,
        type: 'started'
      }).success
    ).toBe(false)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'delta',
        delta: 'ok',
        unexpected: true
      }).success
    ).toBe(false)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'completed',
        lastContentSequence: 10,
        contentScalarCount: 1,
        content: '😀'
      }).success
    ).toBe(false)
  })

  it('enforces output limits in Unicode scalars', () => {
    const atLimitAstral = '😀'.repeat(AI_OUTPUT_LIMIT)

    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'delta',
        delta: atLimitAstral
      }).success
    ).toBe(true)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'completed',
        lastContentSequence: 10,
        contentScalarCount: AI_OUTPUT_LIMIT
      }).success
    ).toBe(true)
    expect(
      actionStreamEventSchema.safeParse({
        ...orderedBase,
        type: 'completed',
        lastContentSequence: 10,
        contentScalarCount: AI_OUTPUT_LIMIT + 1
      }).success
    ).toBe(false)
  })

  const snapshot = (overrides: Record<string, unknown> = {}) => ({
      sessionId: 'session-1',
      sessionGeneration: 1,
      requestId: crypto.randomUUID(),
      requestGeneration: 1,
      actionId: 'translate',
      selection: {
        text: 'hello',
        sourceApp: { name: 'Test', bundleId: null },
        anchor: { kind: 'cursor' as const, x: 10, y: 20 },
        direction: 'unknown' as const,
        isFullscreen: false
      },
      status: 'streaming' as const,
      content: '',
      contentScalarCount: 0,
      lastSequence: 0,
      lastContentSequence: 0,
      handshakeGeneration: 1,
      generationNotice: undefined,
      providerId: undefined,
      modelId: undefined,
      errorMessage: '',
      retryable: false,
      pinned: false,
      ...overrides
    })

  it('validates paired optional routes and exact snapshot scalar counts', () => {
    expect(resultSessionSnapshotSchema.safeParse(snapshot()).success).toBe(true)
    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({
        providerId: 'openai-compatible',
        modelId: 'model-1'
      })).success
    ).toBe(true)
    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({ providerId: 'provider-only' })).success
    ).toBe(false)
    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({ modelId: 'model-only' })).success
    ).toBe(false)

    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({
        content: '😀𠮷',
        contentScalarCount: 2,
        lastContentSequence: 2,
        lastSequence: 2
      })).success
    ).toBe(true)
    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({
        content: '😀𠮷',
        contentScalarCount: 3,
        lastContentSequence: 2,
        lastSequence: 2
      })).success
    ).toBe(false)

    const forgedMarkdownCount = `# heading\n${'😀'.repeat(16_384)}`
    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({
        content: forgedMarkdownCount,
        contentScalarCount: 16_384,
        lastContentSequence: 2,
        lastSequence: 2
      })).success
    ).toBe(false)
  })

  it('validates persisted notices and rejects unknown notice fields', () => {
    const generationNotice = {
      code: 'THINKING_CONTROL_FALLBACK',
      message: '服务商不支持当前控制。'
    }
    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({ generationNotice })).success
    ).toBe(true)
    expect(
      resultSessionSnapshotSchema.safeParse(snapshot({
        generationNotice: { ...generationNotice, unexpected: true }
      })).success
    ).toBe(false)
  })

  it('accepts only privacy-safe renderer marker values', () => {
    expect(resultRendererMarkerSchema.safeParse('firstDomCommit').success).toBe(true)
    expect(resultRendererMarkerSchema.safeParse('firstPresentationOpportunity').success).toBe(true)
    expect(resultRendererMarkerSchema.safeParse('firstDelta').success).toBe(false)
  })
})

describe('screen capture settings', () => {
  it('accepts an unset pin shortcut while requiring a capture shortcut', () => {
    const settings = structuredClone(DEFAULT_PUBLIC_SETTINGS);
    expect(settings.capture.pasteShortcut).toBe('');
    expect(publicSettingsSchema.safeParse(settings).success).toBe(true);
    expect(settingsUpdateSchema.parse({ capture: settings.capture }).capture).toEqual(settings.capture);
    expect(settingsUpdateSchema.safeParse({ capture: { ...settings.capture, pasteShortcut: ' ' } }).success).toBe(false);
    settings.capture.shortcut = '';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(false);
  });
  it('rejects shortcuts that collide after modifier reordering', () => {
    const settings = { ...structuredClone(DEFAULT_PUBLIC_SETTINGS) };
    settings.capture.pasteShortcut = 'PrintScreen';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(false);
    settings.capture.pasteShortcut = 'Control+Shift+KeyP';
    settings.capture.shortcut = 'Shift+Control+KeyP';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(false);
    settings.capture.shortcut = 'PrintScreen';
    settings.quickAskShortcut = settings.capture.shortcut;
    expect(publicSettingsSchema.safeParse(settings).success).toBe(false);
  });
  it('accepts standalone functional keys and standard modifier combinations', () => {
    const settings = { ...structuredClone(DEFAULT_PUBLIC_SETTINGS) };
    settings.capture.shortcut = 'PrintScreen';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(true);
    settings.capture.shortcut = 'F12';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(true);
    settings.capture.shortcut = 'Super+Shift+KeyS';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(true);
    settings.capture.shortcut = 'Control+PrintScreen';
    expect(publicSettingsSchema.safeParse(settings).success).toBe(true);
  });
  it('validates annotation preferences and preserves explicit capture updates', () => {
    const capture = { ...DEFAULT_PUBLIC_SETTINGS.capture, color: '#12abEF', lineWidth: 32 };
    expect(settingsUpdateSchema.parse({ capture })).toEqual({ capture });
    expect(settingsUpdateSchema.safeParse({ capture: { ...capture, lineWidth: 0 } }).success).toBe(false);
    expect(settingsUpdateSchema.safeParse({ capture: { ...capture, color: '#abcd' } }).success).toBe(false);
  });
});
