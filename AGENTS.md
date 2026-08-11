# RunCove Project Instructions

## Positioning

RunCove is a Windows-first local development runtime center. It monitors local
ports, associates trusted project configurations with processes, launches and
stops development commands, captures session logs, and restores the last run
set on demand. The existing cross-platform port-inspection CLI remains
supported through `runcove` and the legacy `portpeek` command.

## Structure

```text
runcove/
|- AGENTS.md
|- HANDOFF.md
|- notes.md
|- Cargo.toml
|- src/                    # Shared Rust core and CLI entrypoints
|- tests/                  # Rust integration and CLI regression tests
`- apps/desktop/
   |- src/                 # React/TypeScript frontend
   `- src-tauri/           # Tauri IPC, persistence, and process management
```

## Development And Verification

Run commands from the repository root unless a command says otherwise.

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets

cd apps/desktop
npm run lint
npm run typecheck
npm test -- --run
npm run build
npm run e2e
npm run tauri build
```

Focused frontend and Rust test commands may be used during development, but the
full checks above are required before completion.

## Generated Output

- Root Rust artifacts: `target/`
- Desktop frontend artifacts: `apps/desktop/dist/`
- Tauri artifacts: `apps/desktop/src-tauri/target/`
- Installed dependencies: `apps/desktop/node_modules/`
- Playwright artifacts: `apps/desktop/output/playwright/e2e-artifacts/`
- Runtime database and state: the Tauri application-local data directory

Do not retain build artifacts, caches, runtime databases, captured logs, or
temporary fixtures in Git. Do not read or modify project `.env` files.

## Engineering Rules

- Keep frontend access narrow: filesystem, database, scanning, and process
  actions must go through typed Tauri commands or channels.
- Treat project/port association as trusted only when it is managed by RunCove
  or explicitly confirmed by the user. Inference remains a suggestion.
- Store launch commands as executable plus argument array and working directory;
  do not persist interpolated shell command strings.
- Never elevate automatically. Surface permission failures explicitly.
- Preserve legacy CLI flags, JSON fields, and exit-code behavior.
- Do not edit CI/release workflows, commit, push, rename the remote repository,
  or publish without explicit authorization.

## Handoff

Update `HANDOFF.md` after each completed milestone, important decision, or
resolved blocker. Record final decisions, verification evidence, and unresolved
issues in `notes.md`.
