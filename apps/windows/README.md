# Windows App Sources

This directory contains Windows-owned code and assets:

- `src/selection.rs`
- `renderer/startup/`
- `icons/`
- `scripts/verify-artifacts.mjs`

Build entry points remain in shared locations:

- `src-tauri/src/selection.rs` imports `src/selection.rs` from this directory.
- `src/renderer/startup/index.html` and `src/renderer/startup/main.ts` are thin build shims that load `renderer/startup/`.
- `src-tauri/tauri.windows.conf.json` references the icon assets in this directory.

Shared toolbar, result window, settings UI, model requests, and most Rust runtime code remain in `src/` and `src-tauri/src/`.

## Application icons

`icons/icon.ico` supplies both the executable's embedded icon and Tauri's
default window icon; `icons/32x32.png` supplies the notification-area icon.
`src-tauri/build.rs` explicitly tracks these files so replacing an icon also
rebuilds native resources when Cargo artifacts are reused. Installer icons
alone do not update the executable's embedded icon.

After installing a build with new icons, fully exit Popper from its tray menu
and launch the installed version again. If an existing pinned taskbar shortcut
still displays the old icon, unpin it and pin the newly launched app again.

## Optional tray and reopening settings

The sidebar's **启用托盘** switch defaults to on, including for existing settings
files. Save settings to apply it immediately; the preference persists across
restarts. When off, startup skips tray creation and saving removes any existing
tray resource. Closing settings still keeps selection assistance running.

Launching `Popper.exe` again opens, restores, and focuses the running instance's
settings panel, regardless of the tray preference. An early second launch is
queued until the primary instance has finished setting up its windows.

Windows acceptance: disable and save, confirm the notification-area icon
disappears, close settings, and launch the executable again. Confirm settings
opens with the switch off and only one Popper process owns the selection hooks.
Restart to check no tray is created; re-enable and save to check it returns.
Repeat toggling and check a tray menu action fires only once. Also verify a
minimized settings window is restored by another launch.

## Mouse/UIA selection offset detection

Windows records the latest completed left-button gesture without querying UIA
in the low-level hook. A right-button hold requests capture; the isolated COM
helper then checks the saved drag endpoints or multi-click anchors against the
UIA selection. There
are no background selection probes or additional settings.

The detector uses one TextPattern, `RangeFromPoint`, and `CompareEndpoints`.
It allows two UIA character units on either side of each mouse insertion point,
uses text order for forward/reverse selections, and supplements endpoint checks
with the first/last character rectangles. Geometry is advisory: missing
rectangles and mouse endpoints in whitespace do not imply an offset.

| Health | Capture policy |
| --- | --- |
| `NORMAL` | Continue the existing native provider order; validate the actual UIA range before returning it. |
| `OFFSET_DETECTED` | Skip native provider results and use the existing guarded clipboard fallback. |
| Applicable `SUSPICIOUS` | Use the same guarded clipboard fallback, including one mismatched endpoint, reliable geometry contradictions, multiple ranges, an unstable selection, distinct drag coordinates mapping to the same UIA insertion point for a nonempty selection, or multi-click containment/mapping failures. |
| Non-applicable `SUSPICIOUS` | Keep the existing provider order; Shift-click, plain clicks and identical physical endpoints labeled as an ordinary drag are not applicable. |
| `UIA_UNAVAILABLE` | Keep the existing provider order; cancellation/context changes and password controls abort capture. |

For `EqualPointRanges`, distinguish equal physical mouse coordinates from
identical UIA point mappings. A real drag with distinct screen endpoints and a
nonempty selected range (confirmed by a bounded `GetText(1)` probe) is
applicable `SUSPICIOUS` if `RangeFromPoint` reports
one insertion position for both ends (observed in Zotero PDF Reader). The initial
capture gate and candidate revalidation both route it to guarded copying before
UIA/Legacy/MSAA can return the related selection. Direction, endpoint validity
and geometry remain unknown, with confidence 0; this does not claim a measured
offset. Collapsed/caret selections remain `UIA_UNAVAILABLE` with
`EmptySelection`, and identical physical endpoints of an ordinary drag remain
non-applicable.

Double-click word selection, triple-click paragraph selection and subsequent
word/paragraph dragging use a separate **click containment** strategy. The same
stable-selection sampling, 60-second context checks and shared phase budget
apply. A `MultiClick` with identical physical coordinates is valid: its click
should lie inside the selected word/paragraph, rather than equal either of the
selected range's endpoints. Both anchors are checked when the multi-click moves.

Containment uses a cloned selection expanded by up to two character units at
both ends, including document boundaries; the source range is not modified.
The selected range's rectangles provide supplementary geometry for each click,
using the existing pixel tolerance and reliable-character anchor check. Missing
rectangles or whitespace geometry alone remain advisory. Matching clicks report
`NORMAL / ClickMatchesSelection`; an outside click reports applicable
`SUSPICIOUS / ClickOutsideSelection`. Reliable geometry contradictions also
use guarded copying. Once nonempty selected text is confirmed, failed point
mapping/comparison reports applicable `SUSPICIOUS / ClickMappingUnavailable`
and routes to the existing guarded-copy entry. Empty selection or unavailable
text keeps the existing provider route; cancellation, protection, context
changes and deadlines retain their existing handling.

The optional internal health field `clickValidation` contains down/up physical
points, containment, geometry validity, pixel distances and the selected
rectangles. Drag endpoint fields remain unknown for click validation. The DEV
panel shows click correspondence and includes the evidence in copied reports;
renderer selection payloads and settings are unchanged. Candidate UIA ranges
are revalidated with the same multi-click strategy before returning text.

A saved gesture expires after 60 seconds. New left presses, real selection/edit
keyboard input, scrolling, unrelated foreground changes, stopping the monitor
and restarting its hook thread invalidate it. The triggering right press
preserves it. Captures without a saved mouse gesture (including keyboard
selections) do not claim mouse endpoint validation.

Selection samples are scheduled at 0, 28 and 56 ms and require two consecutive
matching ranges. Lookup and endpoint detection share a 180 ms phase deadline,
which also counts against the existing capture deadline. Individual third-party
COM calls can still block; the existing helper-process timeout remains the hard
isolation boundary. Detection does not access the clipboard, inject keys, select
or modify a host range. The existing clipboard snapshot, safety gates, freshness
checks and restore/recovery transaction are reused unchanged.

Hook, event worker and COM helper threads establish Per-Monitor V2 DPI awareness.
Hook points, physical cursor positions, UIA hit-testing and rectangle distances
stay in physical virtual-desktop pixels, including negative monitor coordinates.
The health result's `confidence` means evidence completeness (0.7 for usable
endpoint comparisons, plus 0.15 per usable geometry check), not an accuracy
probability. It does not control routing.

### Diagnostics

Set `POPPER_SELECTION_TRACE=1` before starting Popper, or create the existing
`%LOCALAPPDATA%\Popper\selection-trace.enabled` marker. The existing
`selection-diagnostic.log` then includes detector status/reason, applicability,
mouse endpoints, provider identity, direction, stability, endpoint validity,
character rectangles/distances and the chosen capture method. Ordinary trace
output does not contain selected text, nearby text, window titles or AutomationId.

**Runtime DEV Debug:** use the **DEV Debug** switch at the bottom left of
Windows settings. It takes effect immediately for the running app, including
already-warmed helpers. It is never saved in settings and always starts **off**
on a new app launch. Enabling it records input history and text-debug reports
for subsequent captures. Disabling it stops diagnostics, clears in-memory
reports/input history and hides existing panels; it does not delete logs already
written to disk. Old in-flight reports are discarded even if debug is re-enabled.
The former `POPPER_SELECTION_DETECTION_DEBUG` environment override is no longer
used. Ordinary trace can still be enabled independently of DEV Debug.

While enabled, result windows with a diagnostic report include a collapsed
**DEV · 选区偏移诊断** panel.
Expand it and click **复制诊断报告** after reproducing the incorrect selection.
The report is scoped to that result's original selection timestamp (not the
latest global capture); only its own result window can request it. The parent
keeps at most 16 reports and the latest 48 input events. Helper steps are capped
at 192 (first 128 and last 64) and include relative elapsed times; truncated
reports are marked.

Reports include the saved left gesture and tracker invalidations, DPI readiness,
hit-test/provider lookup, ancestor prefilter outcomes, failed-call HRESULTs,
endpoint comparisons (order only), requested/actual character movement,
rectangles, stability, and initial **and** revalidated health. They distinguish
entering the guarded-copy route from successfully returning clipboard text.
Existing clipboard protection, injection and restoration are unchanged.

The same report is written to
`%LOCALAPPDATA%\Popper\selection-diagnostic.log`. Empty, failed or cancelled
captures can be diagnosed there even when no result window opens. Selected and
nearby text, window title and AutomationId are bounded to 512 Unicode scalars;
nearby probes request at most 64 UTF-16 units. JSON escapes line breaks. Selected
text is present only in reports collected while DEV Debug is enabled.

For the reported PDF failure: drag over the word, hold the right button, then
copy the report before making another selection. Compare the visually selected
word with `selectionText`, `initialHealth`, `health`, `provider`, and
`fallbackAttempted`. Check `inputEvents` for missing/invalidated drag context,
`uia.find-text-pattern.stopped` for exhausted lookup budget, and
`uia.CompareEndpoints` / `uia.GetBoundingRectangles` for a provider whose point
mapping agrees with its incorrect selection. A report is diagnostic evidence;
`NORMAL` still does not prove visual correctness.

To verify the switch: start Popper and confirm DEV Debug is off with no panel
or new text-debug records. Enable it without saving settings, reproduce a
selection and copy its report. Disable it with a result window still open and
confirm its panel disappears; make another selection and verify no new debug
report is recorded. Reopen settings in the same app run to confirm the current
switch state, then restart Popper to confirm it is off again. Detector health
checks and guarded-copy routing continue to operate with DEV Debug off.

The internal helper protocol is version 3 and carries mouse context and
`SelectionHealthResult`. Renderer selection events and persisted settings are
unchanged.

### Verification

For multi-click acceptance, double-click a word and triple-click a paragraph,
then hold the right button. Verify `ClickMatchesSelection` for correct UIA
mappings. Reproduce the Zotero PDF offset: a click that maps to unrelated text
should show `ClickOutsideSelection`, `fallbackAttempted: true`, and actual
`copyInjected` when the guarded shortcut is sent. Also cover word/paragraph
dragging in both directions, document edges, empty selections, missing
rectangles, cancelled input and mixed DPI. Preserve the pre-capture clipboard
contents to verify restoration after a successful copy fallback.


The detector and tracker are independent of Windows and expose a fakeable UIA
adapter. Their tests run in the regular Rust suite on Windows/macOS and cover
forward/backward offsets, inclusive tolerance, document boundaries, text-order
direction, geometry gaps, multiple/unstable selections, cancellation and context
invalidation. Windows tests also verify helper request/result serialization and
input-state integration. `pnpm verify:windows` remains the complete Windows
verification entry point.

On a Windows desktop, test Notepad, Chrome/Edge, Firefox, Zotero's main UI, Note
Editor and PDF Reader, and VS Code at 100%, 125%, 150% and 200% scaling, plus
mixed-DPI/negative-coordinate monitors. For each application:

1. Drag-select a known phrase, then hold right-button for 250 ms. Confirm the
   returned text matches the visual selection and normal drags avoid copying.
2. Repeat in reverse and across three lines, including RTL text where available.
3. End near a character boundary or outside text; confirm tolerance/missing
   geometry do not produce a false offset.
4. Try double/triple-click, Shift-click and keyboard selection; confirm ordinary
   capture still works without strict drag endpoint classification.
5. Change the selection, scroll, switch windows or wait over 60 seconds before
   triggering; confirm stale endpoints are not reused.
6. Reproduce the Zotero PDF offset with trace enabled. A reliable mismatch must
   select clipboard routing even if Legacy UIA/MSAA report the same wrong text.
   Check the original clipboard is restored and user input cancels capture.

`RangeFromPoint` and rectangle mapping are still supplied by the application.
If these mappings and `GetSelection` share the same defect, the detector may miss
it. `NORMAL` is evidence of consistency with the available signals, not proof of
visual correctness. Real Zotero regression testing remains necessary.
