# Contributing to OmniSearch-Local

Thanks for your interest in contributing! This project is a Tauri v2 desktop app with a Rust backend and a React/TypeScript frontend.

## Development setup

1. Install the [prerequisites](README.md#prerequisites): Node.js, pnpm, Rust, and Tauri's platform-specific dependencies.
2. Fork and clone the repo, then:
   ```bash
   pnpm install
   pnpm tauri dev
   ```
3. Frontend code lives in `src/`, backend (Rust) code lives in `src-tauri/src/`.

## Before opening a PR

Please make sure these all pass locally:

```bash
# Frontend
pnpm exec tsc --noEmit
pnpm exec vite build

# Backend
cd src-tauri
cargo test
cargo clippy --all-targets
```

The CI workflow runs the same checks on every PR, so it's faster to catch issues locally first.

## Code style

- **Rust:** standard `rustfmt` formatting (`cargo fmt`), and code should be `clippy`-clean. Prefer explicit error messages (`Result<T, String>` with context) over `.unwrap()` outside of tests.
- **TypeScript/React:** functional components, no class components. Match the existing project structure - `src/components/` for UI, `src/lib/` for non-UI logic (search, indexing, embeddings, etc.), one React context per cross-cutting concern (see `indexing-context.tsx`, `ollama-context.tsx`).
- Keep new dependencies minimal - this project deliberately favors small, focused crates/packages over heavier alternatives (e.g. a hand-rolled BM25 implementation instead of pulling in a full search engine, `zip` + `quick-xml` instead of a full docx-editing library).

## Adding a new file type

File extraction lives in `src-tauri/src/ingest.rs`. To add a new format:
1. Add its extension(s) to the relevant constant (or a new one) near the top of the file.
2. Add it to the `is_supported` check.
3. Write an `extract_*_text` function and wire it into `extract_document_text`'s dispatch.
4. Add a unit test - for binary formats like `.docx`/`.pptx`, see `write_zip_fixture` in the test module for how to construct a minimal fixture without needing a real sample file.

## Reporting bugs / requesting features

Please use the issue templates - they help make sure reports include what's needed to reproduce or evaluate them. Screenshots or a short screen recording are especially helpful for UI issues.

## Questions

Open a [discussion or issue](../../issues) - happy to help you get oriented in the codebase.
