# silent_disco

[![CI](https://github.com/ekkus93/silent_disco/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/ekkus93/silent_disco/actions/workflows/ci.yml)
[![Desktop CI](https://github.com/ekkus93/silent_disco/actions/workflows/desktop-ci.yml/badge.svg?branch=master)](https://github.com/ekkus93/silent_disco/actions/workflows/desktop-ci.yml)
[![Source file line limit](https://github.com/ekkus93/silent_disco/actions/workflows/source-file-line-limit.yml/badge.svg?branch=master)](https://github.com/ekkus93/silent_disco/actions/workflows/source-file-line-limit.yml)

Silent Disco for Android

## Where to look first

> **Note:** the one-line description above predates the migration to a shared
> Rust core with a Tauri desktop host. Treat the documents below as
> authoritative over this README.

- **[docs/AUDIO_PLAYBACK_STATE_2026-08-10.md](docs/AUDIO_PLAYBACK_STATE_2026-08-10.md)**
  — current state of listener audio playback: what works, what is still
  wrong and in what priority order, how to run and measure it on a physical
  device, and the pitfalls (and dead ends) that cost the most time. Read
  this before touching the audio path, clock sync, or the render ring.
  Multi-listener playback is **not** yet validated on real hardware; that
  document says what is known and what still has to be measured.
- `docs/SILENT_DISCO_RUST_CORE_ARCHITECTURE_SPEC.md` and
  `docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md` — shared-core architecture
  and migration plan.
- `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md` and
  `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md` — desktop companion host.
- `memory.md` — dated session-by-session record of decisions, failures, and
  real-device results.
