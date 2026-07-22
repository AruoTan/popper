# macOS App Sources

This directory contains macOS-owned code and assets:

- `native/selection_bridge.h`
- `native/selection_bridge.mm`
- `icons/icon.icns`
- `icons/tray-template.png`
- `scripts/verify-artifacts.mjs`

Build entry points remain in shared locations:

- `src-tauri/build.rs` compiles the native bridge from this directory.
- `src-tauri/tauri.conf.json` references the macOS icon here.
- `src-tauri/src/runtime.rs` loads the tray template from this directory.

Cross-platform renderer, settings, action logic, model requests, and shared Rust runtime stay in `src/` and `src-tauri/src/`.
