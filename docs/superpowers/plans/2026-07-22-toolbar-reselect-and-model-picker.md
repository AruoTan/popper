# Toolbar Re-select Reliability and Selective Model Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix intermittent missing toolbar after closing a result on macOS, and replace destructive “同步模型” with fetch → multi-select → draft merge plus a vertical reorder list for curated models.

**Architecture:** (1) Runtime defers host selection clear after result cleanup, cancels clear on new selection, arms same-text suppress once, restores source activation after AI/ask reveal. (2) Settings uses list-only model fetch + modal multi-select merge into provider.models draft; selected models render as a vertical sortable list. No settings schema version bump.

**Tech Stack:** Tauri 2, Rust (`runtime.rs`, `actions.rs`), React + TypeScript settings UI, Vitest + Testing Library, dnd-kit, existing `window.textLens` bridge.

**Spec:** `docs/superpowers/specs/2026-07-22-toolbar-reselect-and-model-picker-design.md`

## Global Constraints

- macOS is the primary repro; keep fixes cross-platform-safe; do not regress Windows toolbar/result dismiss.
- Keep `SAME_TEXT_SELECTION_SUPPRESS_MS` at **2000**; do not lengthen to “fix” different-text failures.
- Deferred clear default **200ms** (range 150–300ms); cancel if a new selection is accepted before fire.
- Model apply merges into **draft** only; user uses global **保存设置** (banner reminds).
- Do **not** auto-write full remote catalog into settings; stop primary UI use of full-replace sync.
- Manual model IDs not in remote list must be preserved on apply.
- No request AbortController for model fetch in v1 (timeout only).
- No schema / `SETTINGS_VERSION` bump unless absolutely required (prefer none).
- Touch only files listed per task; run targeted tests after each task; full `pnpm test` at integration.
- Prefer worktree branch: `fix/toolbar-reselect-and-model-picker`.

## File map

| Area | Primary files |
|------|----------------|
| Deferred clear + suppress + activation | `src-tauri/src/runtime.rs` (+ unit tests in same file `#[cfg(test)]`) |
| Host clear API (if needed for cancel token) | `src-tauri/src/selection.rs` / bridge only if API change required — prefer no native change |
| List models IPC | `src-tauri/src/actions.rs`, `runtime.rs`, `lib.rs`, `models.rs` (result type if new) |
| Bridge / IPC names | `src/shared/ipc.ts`, `src/renderer/lib/tauriBridge.ts`, `tauriBridge.test.ts` |
| Merge helpers | Prefer pure helpers in `src/renderer/settings/settingsForm.ts` (or new `providerModels.ts` if form file is crowded) |
| Settings UI | `SettingsApp.tsx`, `settings.css`, `SettingsApp.test.tsx` |

---

### Task 1: Deferred cancelable host clear + single suppress arm

**Files:**
- Modify: `src-tauri/src/runtime.rs`
- Test: unit tests in `src-tauri/src/runtime.rs` `#[cfg(test)]` module (existing patterns near `same_text_selection_*` tests)

**Interfaces:**
- Consumes: `HostSelectionClear`, `SelectionMonitor::clear_matching_text`, `SAME_TEXT_SELECTION_SUPPRESS_MS`, existing `cleanup_result_session`
- Produces:
  - `const HOST_SELECTION_CLEAR_DELAY_MS: u64 = 200;`
  - `PendingHostClear { token: u64, text: String, bundle_id: Option<String> }` (or equivalent)
  - `fn schedule_host_selection_clear(&self, clear: HostSelectionClear)`
  - `fn cancel_pending_host_selection_clear(&self)` (or bump token)
  - `cleanup_result_session` no longer calls clear synchronously
  - `handle_selection` cancels pending clear when a selection is accepted for presentation

- [ ] **Step 1: Add failing unit tests for pure clear scheduling helpers**

If clear scheduling is hard to unit-test with full `RuntimeState`, extract pure helpers (same style as `should_suppress_same_text_selection`):

```rust
// Example pure API (adjust names to fit file):
fn should_fire_pending_host_clear(scheduled_token: u64, current_token: u64) -> bool {
    scheduled_token == current_token
}
```

Also add tests documenting intended cleanup behavior in comments + test:

1. `should_suppress_same_text_selection` still exact-match + expiry (existing may already cover).
2. New: double arm of suppress extends deadline (document current `arm_same_text_selection_suppress`); after Task 1 cleanup path should only arm once per close — assert via a small helper `arm_count` is not required if you refactor cleanup to call arm once and destroy path only calls cleanup.

- [ ] **Step 2: Run Rust tests — expect new assertions to fail or not yet compile**

Run:  
`cd src-tauri && cargo test --lib same_text_selection -- --test-threads=1`  
and/or  
`cargo test --lib runtime:: -- --test-threads=1`  
(If `frontendDist` missing, create stub `dist/index.html` as previously.)

- [ ] **Step 3: Implement deferred clear**

In `RuntimeState`:

1. Add `pending_host_clear_token: AtomicU64` (or Mutex generation).
2. `schedule_host_selection_clear(text, bundle_id)`:
   - bump token
   - clone token
   - `tauri::async_runtime::spawn` + `tokio::time::sleep(Duration::from_millis(HOST_SELECTION_CLEAR_DELAY_MS))` (mirror `schedule_resize_persist`)
   - on wake: if token still current, call `SelectionMonitor::clear_matching_text`
3. `cancel_pending_host_selection_clear`: bump token so in-flight tasks no-op.
4. In `cleanup_result_session`:
   - remove session meta
   - **arm suppress once** with session text
   - **schedule** clear instead of sync clear
5. Audit `WindowEvent::Destroyed` / close paths: if they both arm suppress and call cleanup, ensure suppress is not re-armed twice for the same close (cleanup only, or destroy only — pick one path).
6. In `handle_selection`, after all early-returns that skip presentation, when about to present toolbar: `cancel_pending_host_selection_clear()`.

Do not change suppress duration.

- [ ] **Step 4: Run Rust tests — expect pass**

Run: `cargo test --lib runtime:: -- --test-threads=1`  
Also: `cargo test --lib same_text -- --test-threads=1`

- [ ] **Step 5: Commit**

```
fix(runtime): defer host selection clear after result close
```

---

### Task 2: Restore source activation after AI / ask result reveal

**Files:**
- Modify: `src-tauri/src/runtime.rs` (`run_action` AI and ask branches ~2210–2270)
- Test: prefer a focused unit test if activation is pure-mockable; otherwise document manual check and keep logic minimal

**Interfaces:**
- Consumes: `restore_source_app_activation(&app, bundle_id)` (already used on copy success)
- Produces: same call after successful AI/ask result reveal (alongside existing suppress + clear_and_hide selection)

- [ ] **Step 1: Locate copy path pattern**

Copy success (~2170): arms suppress + `restore_source_app_activation`. Mirror for:

- `opens_result_without_generation` success branch after reveal Ok
- AI `is_ai()` success branch after reveal Ok

Use `selected_text` / selection payload `source_app.bundle_id` already in scope.

- [ ] **Step 2: Implement restore calls**

```rust
let _ = restore_source_app_activation(&app, &selection.payload.source_app.bundle_id);
// For branches that already moved selection into session, use saved bundle_id string.
```

Keep soft-restore semantics (no ActivateAllWindows flash).

- [ ] **Step 3: Compile / test**

Run: `cargo test --lib runtime:: -- --test-threads=1`

- [ ] **Step 4: Commit**

```
fix(runtime): restore source app focus after AI result reveal
```

---

### Task 3: List-only model IPC + bridge

**Files:**
- Modify: `src-tauri/src/actions.rs` — add `list_provider_models` (or public wrapper around `fetch_models` without write)
- Modify: `src-tauri/src/runtime.rs` — `#[tauri::command] list_provider_models`
- Modify: `src-tauri/src/lib.rs` — register command
- Modify: `src-tauri/src/models.rs` — reuse `ConnectionTestResult` **or** add thin `ListModelsResult { ok, models, message?, status? }` (prefer reuse `ConnectionTestResult` if shape matches)
- Modify: `src/shared/ipc.ts` — `listProviderModels: 'list_provider_models'`
- Modify: `src/renderer/lib/tauriBridge.ts` — `listProviderModels(providerId)`
- Modify: `src/renderer/lib/tauriBridge.test.ts`
- Optional: leave `sync_provider_models` implemented but unused by UI

**Interfaces:**
- Consumes: `ActionService::fetch_models`
- Produces:

```ts
// Bridge
listProviderModels?(providerId: string): Promise<ProviderConnectionTestResult>
// ok:true → models[]; ok:false → message
```

Rust:

```rust
pub async fn list_provider_models(
    state: State<'_, RuntimeState>,
    provider_id: String,
) -> Result<ConnectionTestResult, String> // or dedicated type
```

- [ ] **Step 1: Failing bridge test**

Add test: `listProviderModels` invokes `list_provider_models` and parses models array / error message (mirror `syncProviderModels` tests).

- [ ] **Step 2: Run vitest — expect fail**

Run: `pnpm exec vitest run src/renderer/lib/tauriBridge.test.ts`

- [ ] **Step 3: Implement Rust + bridge**

- `actions.list_provider_models` = same as `test_provider_connection` (fetch only).
- Register command; wire bridge + ipc constants.
- Do not write settings.

- [ ] **Step 4: Tests pass**

Run:  
`pnpm exec vitest run src/renderer/lib/tauriBridge.test.ts`  
`cargo test --lib -- --test-threads=1` filtered if any Rust tests added

- [ ] **Step 5: Commit**

```
feat(api): list provider models without writing settings
```

---

### Task 4: Pure merge helpers for selected models

**Files:**
- Create or modify: `src/renderer/settings/providerModels.ts` (preferred new small module) **or** `settingsForm.ts` if tiny
- Create: `src/renderer/settings/providerModels.test.ts` (or extend `settingsForm.test.ts`)

**Interfaces:**
- Consumes: `ProviderModel` type from shared schemas
- Produces:

```ts
export function mergeProviderModelsOnPick(args: {
  previous: ProviderModel[]
  remote: ProviderModel[]
  checkedIds: ReadonlySet<string> | string[]
}): ProviderModel[]
```

**Merge rules (from spec):**

1. Keep previous models that remain checked, in previous order.
2. Append newly checked remote models (not already kept) in remote list order.
3. For ids present in remote, use remote `name` / thinking fields when including them.
4. Previous models not in remote but still checked (manual) stay with previous metadata.
5. Unchecked previous models drop out (caller may run `removeProviderModelFromSettings` side effects separately per removed id).

- [ ] **Step 1: Write failing tests**

Cases:

- Keep order of still-checked previous; append new remote.
- Refresh name from remote for matched id.
- Manual-only id remains when still checked and absent from remote.
- Unchecked previous id removed.
- Empty checked → empty list.
- Duplicate ids not introduced.

- [ ] **Step 2: Run — fail**

Run: `pnpm exec vitest run src/renderer/settings/providerModels.test.ts`

- [ ] **Step 3: Implement `mergeProviderModelsOnPick`**

- [ ] **Step 4: Tests pass**

- [ ] **Step 5: Commit**

```
feat(settings): merge helpers for selective model pick
```

---

### Task 5: Fetch modal multi-select UI (replace 同步模型)

**Files:**
- Modify: `src/renderer/settings/SettingsApp.tsx`
- Modify: `src/renderer/settings/settings.css`
- Modify: `src/renderer/settings/SettingsApp.test.tsx`
- Uses: Task 3 bridge + Task 4 merge helper

**Interfaces:**
- Consumes: `window.textLens.listProviderModels` (fallback: `testProviderConnection` if list missing)
- Produces: UI state for modal; draft update via merge; button label **获取模型**

- [ ] **Step 1: Tests for open / apply / cancel**

In `SettingsApp.test.tsx` (mock `textLens`):

1. Button shows **获取模型** (not 同步模型).
2. Click → after mock list returns → modal/dialog with checkboxes; already-selected pre-checked.
3. Apply with subset → draft models equal merge result; dirty; banner mentions save; **no** call to `syncProviderModels`.
4. Cancel → models unchanged.
5. Fetch error → error banner; models unchanged.

Mock `listProviderModels` (or test connection) with a multi-model list larger than current selection.

- [ ] **Step 2: Run — fail**

Run: `pnpm exec vitest run src/renderer/settings/SettingsApp.test.tsx`

- [ ] **Step 3: Implement modal flow**

1. Replace `syncModels` with `openModelPicker(provider)`:
   - set operation spinner
   - `persist()` first (same as today)
   - call list IPC
   - on ok: set modal state `{ providerId, remoteModels, checkedIds: Set }` initialized with current provider model ids
2. Modal UI (reuse existing dialog patterns in `SettingsApp` / `CustomActionDialog` styles if present):
   - title 选择模型
   - search input filters remote by id/name
   - checkbox list
   - 取消 / 应用所选
3. Apply: `mergeProviderModelsOnPick` → `changeProvider` models; set dirty; close modal; success banner.
4. For each removed id vs previous, call existing remove-side-effect helper if not already covered by replace-models path (`removeProviderModelFromSettings` or action modelId clear when rewriting provider models — follow existing remove chip behavior).
5. Remove primary use of `syncProviderModels`.

- [ ] **Step 4: CSS for modal + searchable list**

Keep within settings design tokens; scrollable list max-height ~40vh; dense rows.

- [ ] **Step 5: Tests green**

Run: `pnpm exec vitest run src/renderer/settings/`

- [ ] **Step 6: Commit**

```
feat(settings): fetch and multi-select provider models
```

---

### Task 6: Vertical sortable selected model list

**Files:**
- Modify: `src/renderer/settings/SettingsApp.tsx` (`SortableModelChip` → row list)
- Modify: `src/renderer/settings/settings.css`
- Modify: `src/renderer/settings/SettingsApp.test.tsx` if selectors change

**Interfaces:**
- Consumes: existing dnd-kit `DndContext`, `SortableContext`, `reorderProviderModels`
- Produces: vertical list with grip + name + optional muted id + remove; `verticalListSortingStrategy`

- [ ] **Step 1: Update tests/selectors if they query chip classes**

Ensure remove still works; empty state copy: `尚无模型，可获取或手动添加。`

- [ ] **Step 2: Implement vertical rows**

Replace `model-chip-list` / `SortableModelChip` with e.g. `model-row-list` / `SortableModelRow`:

- grip handle (same a11y title pattern as actions)
- primary name
- secondary id when `id !== name`
- remove button

Strategy: `verticalListSortingStrategy` (import from `@dnd-kit/sortable`).

- [ ] **Step 3: Style rows**

~28–32px height; full width; hover background; dragging opacity.

- [ ] **Step 4: Tests pass**

Run: `pnpm exec vitest run src/renderer/settings/`

- [ ] **Step 5: Commit**

```
feat(settings): vertical reorder list for selected models
```

---

### Task 7: Integration verify + manual checklist

**Files:**
- Optional: brief note only if README product behavior must mention model picker (skip version bump unless packaging)

- [ ] **Step 1: Full frontend tests**

Run: `pnpm test`  
Expected: all pass

- [ ] **Step 2: Rust tests (settings + runtime)**

Run:  
`mkdir -p dist && test -f dist/index.html || echo '<!doctype html><html></html>' > dist/index.html`  
`cd src-tauri && cargo test --lib -- --test-threads=1`  
(If full lib is long, at minimum: `runtime::` + `settings::` + any new tests.)

- [ ] **Step 3: Manual macOS checklist (from spec)**

1. Translate → wait >2s → different text → toolbar  
2. Translate → immediate different-text drag → toolbar  
3. Translate → same-text micro-drag within 2s → no re-pop  
4. Translate → wait >2s → same text → toolbar  
5. 获取模型 → pick subset → 保存 → restart settings → list persisted  
6. Manual model survives fetch apply  
7. Reorder vertical list + save  

- [ ] **Step 4: Final commit if only docs/comments remain**

```
test: verify toolbar reselect and model picker integration
```

---

## Execution notes

- **Worktree:** create via project worktree convention before coding if main is busy:  
  `.worktrees/fix-toolbar-reselect-and-model-picker` on branch `fix/toolbar-reselect-and-model-picker`.
- **TDD:** each task writes failing tests first where feasible; runtime timer may use pure helpers for unit tests and a thin async wrapper for spawn.
- **Do not** reintroduce full-replace sync as the default button.
- **Windows:** no dedicated repro required in v1; avoid `#cfg(macos)` for deferred clear — apply on all platforms.

## Task dependency graph

```
Task1 (deferred clear)
  └─ Task2 (activation restore)     # can be sequential after 1
Task3 (list IPC)
  └─ Task4 (merge helpers)
       └─ Task5 (modal UI)
            └─ Task6 (vertical list)  # can merge with 5 if small
Task7 (integration)
```

Tasks 1–2 and 3–4 are independent → safe for parallel agents after branch exists; integrate before Task 7.
