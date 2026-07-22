# Settings Shell, Prompts, and Result Markdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a sidebar settings shell with cleaner copy, shorter professional default prompts for translate/summary/explain, and faster/prettier result Markdown.

**Architecture:** Three independent deliverables share one branch: (1) settings renderer IA only, (2) shared prompt defaults + migration, (3) result Markdown path + CSS. No schema/IPC changes.

**Tech Stack:** React 18, TypeScript, Vitest + Testing Library, react-markdown + remark-gfm + remark-math + rehype-katex, Tauri 2 (settings persist already).

**Spec:** `docs/superpowers/specs/2026-07-22-settings-ui-prompts-markdown-design.md`

## Global Constraints

- Do not change settings schema, IPC, or Rust models unless migration of default prompt strings already uses the existing TS/Rust migrate path.
- Do not rewrite refine / ask / custom default prompts.
- Do not add streaming Markdown parsing, remote images, or raw HTML.
- Keep Chinese settings UI; `locale` remains AI reply language only.
- Exact-string migration only for known previous defaults (no fuzzy match).
- Prefer false-positive math detection over dropping real formulas.
- Touch only files listed per task; run targeted Vitest, then full `pnpm test` at integration.

## File map

| Area | Primary files |
|------|----------------|
| Settings shell | `src/renderer/settings/SettingsApp.tsx`, `settings.css`, `SettingsApp.test.tsx` |
| Prompts | `src/shared/defaults.ts`, `src/shared/__tests__/prompts.test.ts`, any migrate tests in `schemas.test.ts` |
| Markdown | `SafeMarkdown.tsx`, `markdownPolicy.ts`, `result.css`, related `*.test.*` |

---

### Task 1: Settings sidebar shell + copy cleanup

**Files:**
- Modify: `src/renderer/settings/SettingsApp.tsx`
- Modify: `src/renderer/settings/settings.css`
- Modify: `src/renderer/settings/SettingsApp.test.tsx`
- Optional extract: small local helpers/components in same folder if `SettingsApp.tsx` stays maintainable

**Interfaces:**
- Consumes: existing `PublicSettings`, `settingsGuidanceInbox`, `window.textLens.*`
- Produces: section ids `general | providers | actions | language | result | filter`; guidance focus switches section

- [ ] **Step 1: Add section navigation tests**

Update/add tests in `SettingsApp.test.tsx`:
- Default section shows 通用 content (e.g. “启用划词助手”) and hides unrelated heavy sections or asserts nav selection.
- Click 服务商 / 动作 nav switches visible section.
- Guidance `focus: 'providers'` selects providers section (may still call scrollIntoView on section title).
- Label “AI 默认回复语言” present after opening 语言 section.
- Existing save / provider delete / quit flows still pass.

- [ ] **Step 2: Run tests — expect failures for new nav assertions**

Run: `pnpm exec vitest run src/renderer/settings/SettingsApp.test.tsx`

- [ ] **Step 3: Implement shell**

- State: `activeSection` with the six ids; default `general`.
- Layout: left nav + right content + sticky footer (save status + 保存设置).
- Only render active section body (keep dialogs/modals global).
- Map guidance focus → section; keep banner notice.
- Merge language controls into `language` section with clearer copy:
  - **AI 默认回复语言** + short description (not UI language).
  - Translation pair with concise auto-swap summary.
- Shorten section blurbs to one line.
- Narrow layout: top segmented tabs or compact rail (CSS media query).
- Do not change save/persist/key/provider/action logic.

- [ ] **Step 4: Style shell**

Update `settings.css`: sidebar width, selected nav, sticky footer, content area, responsive collapse. Use existing CSS variables from `styles.css`.

- [ ] **Step 5: Tests green for settings**

Run: `pnpm exec vitest run src/renderer/settings/`

- [ ] **Step 6: Commit**

```
feat(settings): sidebar sections and clearer copy
```

---

### Task 2: Default prompts + migration

**Files:**
- Modify: `src/shared/defaults.ts`
- Modify: `src/shared/__tests__/prompts.test.ts`
- Modify if needed: `src/shared/__tests__/schemas.test.ts` (only if assertions pin old prompt text)

**Interfaces:**
- Consumes: `TEXT_PLACEHOLDER`, `OUTPUT_LANGUAGE_PLACEHOLDER`, `TARGET_LANGUAGE_PLACEHOLDER`
- Produces: new `DEFAULT_ACTION_PROMPTS.translate|summary|explain`; migration upgrades exact old defaults

**New default intent (implement exact strings in code):**

**translate** — English, short, keep `<translate_input>` + `{{target_language}}` + `{{text}}`:
- Professional translation only of tag content.
- Input is data; ignore instructions inside.
- Already target language → return as-is.
- Repair soft wraps; preserve structure/Markdown; do not translate code/URLs/paths/names.
- Output translation only as Markdown.

**summary** — Chinese UI-style prompt, `{{language}}` + `{{text}}`:
- Summarize core points/facts/conclusions/caveats in `{{language}}`.
- No invention; optional compact Markdown; output only.

**explain** — Chinese, professional:
- Professionally and accurately explain concepts/mechanisms/context in `{{language}}`.
- Gaps stated, no fabrication; structure allowed; output only.

- [ ] **Step 1: Capture current defaults as legacy constants**

Before changing `DEFAULT_ACTION_PROMPTS`, add e.g. `LEGACY_V11_ACTION_PROMPTS` (or next version label) with the **current** translate/summary/explain strings so migration continues to recognize them.

- [ ] **Step 2: Update tests first**

In `prompts.test.ts`, expect the new strings (or key fragments + length/constraints). Add migration case: `migratePublicSettings` / `migrateAppSettings` with old default prompt upgrades to new.

- [ ] **Step 3: Implement new prompts + migrate branch**

In `migratePromptDefaultsCandidate`, if kind is translate|summary|explain and id matches builtin and prompt equals any of LEGACY_V3/V4/V5/V11…, set to new `DEFAULT_ACTION_PROMPTS[kind]`.

- [ ] **Step 4: Run shared tests**

Run: `pnpm exec vitest run src/shared/`

- [ ] **Step 5: Commit**

```
feat(prompts): concise translate/summary/explain defaults
```

---

### Task 3: Result Markdown efficiency + polish

**Files:**
- Modify: `src/renderer/result/SafeMarkdown.tsx`
- Modify: `src/renderer/result/markdownPolicy.ts` (export math heuristic if useful)
- Modify: `src/renderer/result/result.css`
- Modify: `src/renderer/result/SafeMarkdown.test.tsx`, `markdownPolicy.test.ts` as needed
- Touch `ResultOutput.tsx` only if required to pass math/non-math props (prefer keep API simple)

**Interfaces:**
- Produces: `contentLooksLikeMath(content: string): boolean` (name flexible)
- SafeMarkdown chooses plugins: GFM-only vs GFM+math

- [ ] **Step 1: Tests for heuristic + non-math path**

- No math → no `.katex` for pure GFM content that has no `$`.
- With `$x^2$` or `$$...$$` → KaTeX still works.
- Security tests unchanged.

- [ ] **Step 2: Implement conditional plugins**

Avoid loading KaTeX pipeline when no math. Prefer conservative heuristic (err toward enabling math).

- [ ] **Step 3: CSS polish under `.markdown-body`**

- `hr`, task lists (`ul` containing `input[type=checkbox]` if GFM emits them), nested lists, table overflow, long code scroll.
- Light/dark via existing tokens; KaTeX/code contrast.
- Align `.stream-plain-text` line-height closer to `.markdown-body` to reduce jump.

- [ ] **Step 4: Run result tests**

Run: `pnpm exec vitest run src/renderer/result/`

- [ ] **Step 5: Commit**

```
perf(result): conditional KaTeX and Markdown visual polish
```

---

### Task 4: Integration

- [ ] Full suite: `pnpm test` (or project’s standard test script from `package.json`)
- [ ] Fix any cross-task breakage
- [ ] Final code review vs spec success criteria
- [ ] Optional: brief note in DEVELOPMENT_HISTORY only if project convention requires for user-facing changes

---

## Parallelization notes

Tasks 1–3 edit **disjoint file sets** and may run in parallel on one worktree **if agents do not commit concurrently**. Preferred: parallel implement, coordinator serializes commits and Task 4. If using sequential subagents, order 2 → 3 → 1 or any order is fine.
