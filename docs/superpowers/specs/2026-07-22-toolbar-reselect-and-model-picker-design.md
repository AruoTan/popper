# Toolbar re-select reliability and selective model picker

**Date:** 2026-07-22  
**Status:** Approved for planning  
**Branch (planned):** `fix/toolbar-reselect-and-model-picker`  
**Platform focus:** Reproduced on **macOS**; Windows not yet verified. Fixes must stay cross-platform-safe; primary validation on macOS.

## Problem

### A. Toolbar missing after result dismiss (intermittent)

Repro:

1. Select text → toolbar appears  
2. Click 翻译 → result window shows output  
3. Click outside → result closes  
4. Select again → toolbar **sometimes** does not appear  

User confirmation: fails for **both same text and different text** on macOS.

Investigation (code path audit):

- A 2s **same-text suppress** (`SAME_TEXT_SELECTION_SUPPRESS_MS`) is intentional for mouse-up echo after copy / result close. It **cannot** block a different string.
- On result session cleanup, `cleanup_result_session` **synchronously** arms suppress and calls `SelectionMonitor::clear_matching_text` (macOS AX collapse of host highlight). That write often runs on the **same gesture** that blurs the result and starts the next drag, racing the host selection → capture empty / dropped → no toolbar.
- AI/result open arms suppress and hides toolbar but **does not** restore source-app activation (copy path does), so `FrontmostApplication` capture can intermittently fail after Accessory focus churn.
- Suppress may be armed **multiple times** on close (window destroyed + cleanup), extending the 2s window from the last arm.

### B. “同步模型” overwrites the full catalog

Settings → AI 服务商: **同步模型** calls `sync_provider_models`, which fetches up to 200 models and **replaces** the provider’s model list in settings. Users cannot pick a subset; large catalogs flood action dropdowns; manual IDs and curated order are destroyed. Chip wrap + drag is weak for intentional ordering of a small curated list.

## Goals

1. After closing a result by outside click, the **next intentional selection** (same or different text) reliably shows the toolbar on macOS; Windows must not regress.
2. Preserve short same-text suppress only for **mouse-up / micro-drag echo**, not as a general “block reselect” policy.
3. Replace destructive full sync with **fetch → multi-select → merge into selected list**.
4. Keep reorder of **selected** models; prefer a clearer vertical list interaction.
5. Manual model IDs remain supported.

## Non-goals

- Redesign result dismiss modes (blur / pointer leave / manual) product policy.
- Stream-time toolbar changes unrelated to post-result reselect.
- Full virtualized model catalog UI for thousands of remote IDs (cap remains 200).
- Request cancellation / AbortController for model HTTP in v1 (timeout only).
- Windows-only selection rewrites (no platform fork unless a proven Windows-only bug appears).
- Changing `MAX_PROVIDER_MODELS` or settings schema version unless a new field is unavoidable (prefer none).
- Replacing dnd-kit.

---

## 1. Toolbar re-select reliability

### 1.1 Root-cause target

| Rank | Mechanism | Blocks different text? |
|------|-----------|------------------------|
| 1 | Sync `clear_matching_text` during the close click / next drag | Yes |
| 2 | Missing source activation restore after AI result reveal | Yes (intermittent) |
| 3 | Duplicate suppress re-arm on close | Same-text only (extends window) |
| 4 | Capture generation / frontmost edge cases | Yes (residual) |

### 1.2 Design

#### A. Deferred, cancelable host clear

On result session cleanup (`cleanup_result_session` and any equivalent destroy path):

1. Still **arm same-text suppress once** for the session’s original capture text (anti re-pop of the still-highlighted original).
2. **Do not** call `clear_matching_text` synchronously on that path.
3. Schedule clear on a short delay (**200ms** default; acceptable range 150–300ms):
   - If a **new** automatic selection event is accepted (`handle_selection` presents a toolbar) before the delay fires → **cancel** the pending clear.
   - If a newer result session for the same text is active → cancel (stale clear).
   - If shutdown → cancel or no-op safely.
4. When the delayed clear runs, still pass bundle id + text as today; ignore clear failures.

Implementation sketch (Rust):

- Store `pending_host_clear: Mutex<Option<PendingHostClear>>` with generation/token + text + bundle.
- `schedule_host_selection_clear(...)` replaces any previous pending clear for simplicity.
- Timer via existing runtime patterns (`spawn` + sleep, or app-handle async).

#### B. Single suppress arm on close

- Ensure destroy + cleanup do not each re-arm in a way that **extends** the window twice for the same close.
- Preferred: arm suppress **only in `cleanup_result_session`** (or only once when result text is known); other destroy hooks call cleanup without double-arming.
- Keep `SAME_TEXT_SELECTION_SUPPRESS_MS` at **2000** unless tests prove a shorter window still blocks mouse-up echo (do not lengthen).

#### C. Restore source activation after AI / ask result reveal

When translate/summary/explain/ask (etc.) result reveal succeeds (same place copy already arms suppress and hides toolbar):

- Call `restore_source_app_activation` with the selection’s source bundle (soft path already used for copy; avoid focus flash regressions from 0.3.45).

#### D. Observability (lightweight)

Optional debug log (no full selection text): reason tags such as `suppress`, `clear_deferred`, `clear_cancelled`, `frontmost` if already available. Prefer existing log style; not user-facing.

### 1.3 Acceptance (manual + tests)

| # | Scenario | Expect |
|---|----------|--------|
| 1 | Translate → wait >2s → select **different** text | Toolbar shows |
| 2 | Translate → **immediately** drag **different** text | Toolbar shows (regresses clear race) |
| 3 | Translate → outside micro-drag on same highlight within 2s | Toolbar does **not** re-pop at cursor |
| 4 | Translate → wait >2s → re-select **same** text | Toolbar shows |
| 5 | Copy still: success icon + one-click dismiss behavior | No regression |
| 6 | Windows smoke (when available) | No new double-toolbar / stuck hide |

Unit tests (Rust):

- Suppress: different string never suppressed; re-arm extends deadline only when intentionally re-armed.
- Deferred clear: cancelled when a new selection is handled; fires when no new selection.
- Cleanup still arms suppress if clear is deferred/fails.

### 1.4 Risks

| Risk | Mitigation |
|------|------------|
| Host highlight lingers ~200ms | Acceptable; suppress still blocks same-text echo |
| Skip clear → rare same-text re-pop after 2s if host still selected | User reselect or click away; optional second clear only if still matching after delay |
| Focus restore flash | Reuse soft restore used by copy |

---

## 2. Selective model picker

### 2.1 UX

#### Button

- Rename **同步模型** → **获取模型** (or **从服务商添加**; primary copy: **获取模型**).
- Loading: spinner on button; disable while in flight.
- Do **not** auto-write the full remote list into settings.

#### Modal: “选择模型”

Triggered after successful list fetch (persist draft config first so base URL / key are committed, same as today’s pre-sync save).

| Element | Behavior |
|---------|----------|
| Search | Filter by id and name (client-side) |
| List | Checkboxes; show id; name if different; optional thinking badge if metadata present |
| Pre-check | Models already on the provider stay checked |
| Manual-only | Selected models **not** in remote list stay on the main card; modal notes they are not in remote (no forced uncheck) |
| Footer | 取消 · 应用所选 (N) |
| Empty / error | Empty state or banner; previous selection unchanged |
| Cap | Remote list already capped at 200 server-side; show “N 个结果” |

#### Apply merge rules

On **应用所选**:

1. Build `nextModels` from:
   - Previously selected models whose ids remain checked (preserve relative order).
   - Newly checked remote models **appended** in modal list order (or remote order among new ids).
2. For matched remote ids, refresh `name`, `thinkingLevels`, `thinkingCapability` from remote payload.
3. Models unchecked that were previously selected → removed (same side effects as chip remove: clear action `modelId` bindings via existing helper).
4. Models that are selected but **not** in remote (manual) and not shown as unchecked → **keep**.
5. Result goes into **draft** only (`dirty = true`); user saves with global **保存设置** (consistent with manual add / reorder). Do **not** auto-persist full settings unless product later requires it.
6. Close modal; banner e.g. `已选择 N 个模型，请保存设置` when draft changed.

#### Close without apply

Discard temporary remote list; no draft change from the fetch.

#### “Import all” (optional secondary)

**Not** in v1 default chrome. If needed later: overflow “导入全部（最多 200）” with confirmation. Prefer removal of destructive one-click sync over keeping a footgun.

### 2.2 Selected models on the card

Replace wrap **chips** with a **compact vertical list** (~28–32px rows):

```
[≡ grip]  model-name          [×]
          id (muted if ≠ name)
```

- dnd-kit `verticalListSortingStrategy` (same family as action zones).
- Manual add row **above** the list (unchanged).
- Empty: `尚无模型，可获取或手动添加。`
- Count label: `模型（N）`.

### 2.3 Backend / IPC

| Path | Role |
|------|------|
| Existing `fetch_models` | HTTP list only |
| Existing `test_provider_connection` | List without write (usable fallback) |
| Existing `sync_provider_models` | Full replace — **stop using from primary UI** |

**Minimal preferred approach:**

1. Frontend: after `persist()`, call list-only IPC.
2. Prefer a thin command `list_provider_models` that returns `{ ok, models, message?, status? }` without writing settings (wraps `fetch_models`). Keeps “测试连接” messaging separate.
3. If bridge already exposes `testProviderConnection` with models, v1 may use it and still introduce `list_provider_models` for clarity when touching Rust.
4. Leave `sync_provider_models` implemented but unused by UI, or deprecate in a follow-up (no need to delete in v1).

No settings schema version bump required: still `ProviderModel[]` on the provider.

### 2.4 Acceptance

| # | Scenario | Expect |
|---|----------|--------|
| 1 | 获取模型 → check 2 of many → 应用 → 保存 | Only those (+ prior kept) in settings |
| 2 | Manual id not in remote → fetch → apply without touching it | Manual id remains |
| 3 | Uncheck previously selected → apply | Removed; actions unbound if needed |
| 4 | Cancel modal | No draft change from modal |
| 5 | Reorder vertical list → save | Order persisted |
| 6 | Fetch failure | Error banner; selection unchanged |

Frontend tests: merge helper pure functions; modal check/apply; reorder still works. Bridge mock for list IPC.

### 2.5 Risks

| Risk | Mitigation |
|------|------------|
| User forgets global 保存 | Banner reminds; dirty indicator already exists |
| Modal + dirty draft confusion | Persist-before-fetch keeps credentials; merge only dirty draft |
| Large remote list jank | Search filter; simple scrollable list (virtualize only if needed later) |

---

## 3. Implementation order

1. **Toolbar reliability** (P0): deferred clear + single suppress arm + AI activation restore + tests.  
2. **Model list IPC / stop full sync in UI** (P0).  
3. **Modal picker + merge helpers** (P0).  
4. **Vertical selected list reorder** (P1, same PR if small).  
5. Docs: README changelog bullet if version bump is part of release packaging (optional; not required for merge).

## 4. Out of scope reminders

- No change to default prompts / settings shell (already 0.3.52).  
- No KaTeX dynamic import in this work.  
- No Windows-specific selection helper rewrite without a Windows repro.

---

## 5. Open decisions (resolved at approval)

| Topic | Decision |
|-------|----------|
| Primary fix for different-text flake | Deferred cancelable `clear_matching_text` + activation restore |
| Same-text window | Keep 2s; dedupe arm on close |
| Model UX | Modal multi-select; vertical selected list |
| Apply vs save | Merge to draft; global 保存 |
| Import all | Not in v1 primary UI |
| Platforms | macOS primary test; Windows no-regression |

---

## 6. Spec self-review

- [x] No TBD/placeholder sections left for implementers  
- [x] Consistent with investigation (clear race + suppress scope + model full-replace)  
- [x] Scope bounded (two tracks, non-goals listed)  
- [x] Merge rules and cancel/defer semantics specified  
- [x] Acceptance criteria testable  

**Residual ambiguity (acceptable for plan):** exact timer API in Rust (spawn vs delayed task helper) left to implementer matching existing `runtime.rs` patterns.
