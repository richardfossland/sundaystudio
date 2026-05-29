# SundayStudio

Podcast & jingle production for churches — part of the [Sunday suite](../).

The simplest professional podcast producer: 5–8 microphones at once, AI-driven
cleanup and leveling, a generated jingle in under a minute, and a finished
LUFS-normalized MP3 ready for Spotify and Apple Podcasts. Simpler than
GarageBand, friendlier than Audacity, far cheaper than Hindenburg or Audition.

> **Status:** Phase 0.1 (foundation). The app scaffold, design tokens, and the
> first audio smoke tests (device enumeration + WAV writing) are in place. The
> real-time recording engine arrives in Phase 1.

## Stack

Tauri 2 · Rust (cpal + hound, growing into a real-time audio engine) · React 19
· TypeScript · Tailwind CSS v4 · TanStack Query · Zustand · ts-rs bindings.

See [`CLAUDE.md`](./CLAUDE.md) for the product north star and conventions, and
[`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for the audio engine design.

## Develop

```bash
npm install            # install frontend deps (also sets up husky)
npm run tauri dev      # run the desktop app (Rust backend + Vite frontend)
```

## Quality gate

```bash
npm run check          # lint + typecheck + vitest + clippy + cargo test
```

Individually:

```bash
npm run lint           # eslint
npm run typecheck      # tsc --noEmit
npm run test           # vitest (unit/integration)
npm run test:e2e       # playwright (app-shell smoke)
npm run test:rust      # cargo test (audio core: devices + tone)
npm run lint:rust      # cargo clippy -D warnings
```

TypeScript bindings are generated from the Rust types:

```bash
cargo test --manifest-path src-tauri/Cargo.toml export_bindings
```

## License

Proprietary — © Richard Fossland. Part of the Sunday suite.
