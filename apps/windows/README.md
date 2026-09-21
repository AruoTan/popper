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
