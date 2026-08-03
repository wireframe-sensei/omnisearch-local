# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

OmniSearch-Local is a privacy-first, local-only semantic file search desktop app: Tauri v2 (Rust backend) + React/TypeScript/Vite frontend, styled as a Spotlight/Raycast-style floating search bar. All indexing, embedding, and search happen on-device - no data leaves the machine except a one-time embedding-model download and, optionally, requests to a local Ollama instance.

## Commands

### Frontend (run from repo root)

```bash
pnpm install                    # install deps
pnpm tauri dev                  # run the full app (Rust + frontend) in dev mode - this is the only way to actually exercise it
pnpm build                      # tsc typecheck + vite production build (what CI runs)
pnpm exec tsc --noEmit          # typecheck only
pnpm exec vite build            # frontend build only
pnpm tauri build                # full production build (native installers under src-tauri/target/release/bundle/)
```

Running plain `pnpm dev` (vite only, no Tauri shell) is rarely useful here since the app depends on Tauri APIs throughout.

### Rust backend (run from `src-tauri/`)

```bash
cargo test                              # run all unit tests
cargo test <test_name>                  # run a single test, e.g. cargo test extracts_docx_text
cargo clippy --all-targets -- -D warnings   # lint - this exact invocation is what CI enforces
cargo build                             # compile check
```

### CI

`.github/workflows/ci.yml` runs frontend typecheck+build (ubuntu only) and `cargo test` + the clippy command above across a macOS/Windows/Linux matrix. `.github/workflows/release.yml` builds installers via `tauri-action` on `v*` tag pushes (unsigned - expect Gatekeeper/SmartScreen warnings until code signing is configured).

## Architecture

### App shell and lifecycle

The main window (`src-tauri/tauri.conf.json`) is transparent, undecorated, fixed-size, and centered - a floating panel, not a normal window. It's summoned/dismissed via a global hotkey (`Alt+Shift+Space`, registered in `src-tauri/src/lib.rs`). That specific combo was chosen deliberately: `Cmd+Space` is macOS Spotlight and `Cmd+Shift+Space` is 1Password's default quick-access shortcut, so both were avoided; registration failure (e.g. some other app already owns the combo) is logged but non-fatal, so it can't take down the rest of `.setup()` (tray icon, DB init) with it.

On macOS the app runs under `ActivationPolicy::Accessory` (set at the very top of `.setup()`), meaning no persistent Dock icon for the life of the process - it behaves as a tray-only background utility. Closing the window or `Cmd+Q` hides rather than quits: `RunEvent::ExitRequested`'s `code` field is `None` for user-driven exit attempts and `Some(_)` for a programmatic `AppHandle::exit()` call, so the handler only intercepts the former. The tray menu's "Quit" item calls `app.exit(0)` directly, which is the only path that actually terminates the process.

Frontend state lives in two React contexts wrapping the whole app in `App.tsx` (no router - `SearchView`/`SettingsView` are a simple state-driven swap): `IndexingProvider` (`src/lib/indexing-context.tsx`) and `OllamaProvider` (`src/lib/ollama-context.tsx`). Both are mounted above the view swap so their state (indexing progress, Ollama availability) survives navigating between Search and Settings.

### Indexing pipeline

Scanning and text extraction are Rust (`src-tauri/src/ingest.rs`): `walkdir`-based recursive scan, filtered to supported extensions and skipping noise directories (`node_modules`, `.git`, build dirs, and OS-internal folders like `Library`/`AppData` - relevant because Settings offers an "Add Home Folder" shortcut that indexes the whole user directory). Extraction is dispatched per file type: plain text/code read as-is, HTML via `html2text`, RTF via `rtf-parser`, PDF via `pdf-extract`, DOCX/PPTX via raw `zip` + `quick-xml` (pulling text nodes out of the OOXML directly rather than pulling in a full docx-editing crate), XLSX via `calamine`. `src-tauri/src/watcher.rs` runs a `notify`-based recursive watcher and emits `file-changed` events (create/modify/delete) to the frontend.

`src/lib/indexer.ts` orchestrates a **two-phase** pipeline, which is the most important design decision in the codebase to understand before touching this area:

1. **Phase 1 (fast, no ML):** for every changed file, extract → chunk → store chunk *text only* (`upsert_document_chunks_text`). This makes the file immediately keyword-searchable via BM25.
2. **Phase 2 (slow, parallel):** embed every pending chunk across a pool of Web Workers (`src/lib/embedding-pool.ts`, up to 4, capped by `navigator.hardwareConcurrency`), attaching results via `update_chunk_embeddings` as each file's chunks finish - flattened across files (not processed per-file) so a file with few chunks never leaves workers idle waiting on a big file elsewhere.

This split exists so search works immediately after a scan starts rather than only once every file is fully embedded, which used to make large scans (e.g. an entire home folder) feel broken. Chunking (`src/lib/chunk.ts`) tokenizes via the embedding model's own tokenizer and windows at **256 tokens** (not the 500 sometimes cited as a rule of thumb) - `Xenova/all-MiniLM-L6-v2`'s actual trained max sequence length is 256; anything longer would silently truncate.

SQLite storage (`src-tauri/src/store.rs`, `rusqlite` bundled) has `chunks.embedding`/`dim` as **nullable by design**, matching the two-phase split above. Schema is versioned via `PRAGMA user_version` with an in-place migration (`migrate_to_v1`) for databases created before embeddings were nullable - it preserves existing rows/embeddings rather than dropping data. A file's mtime is only recorded once its text is *successfully* stored, so a file whose extraction throws is automatically retried on every subsequent scan - this same property is what keeps the failed-file tracking in `indexing-context.tsx` correct without extra bookkeeping (a fixed file just stops reappearing in the failures list; a still-broken one keeps getting re-reported).

Failed-to-index files are surfaced in Settings (not just logged) with an optional on-demand "Explain" button (`src/lib/error-explainer.ts`) that asks the connected local LLM to translate the raw error into plain English - deliberately click-triggered rather than automatic, since a single bad batch can produce dozens of failures and auto-explaining all of them would mean dozens of unsolicited LLM calls.

### Search

`src-tauri/src/hybrid_search.rs` blends two rankings via **Reciprocal Rank Fusion** (RRF, k=60): a from-scratch BM25 implementation (no SQLite FTS5 - kept in the same Rust query-time path as everything else, avoiding a second index to keep in sync) and cosine similarity (a plain dot product, since embeddings are stored unit-normalized). RRF was chosen specifically to avoid normalizing BM25's unbounded scores against cosine's [-1, 1] range onto a common scale - only each ranking's relative order matters. Chunks with no embedding yet (still in Phase 1) are simply excluded from the semantic ranking but remain fully reachable via BM25; this is the actual mechanism that makes "search available before embedding finishes" work.

`src/lib/hybrid-search.ts` embeds the query (via the same worker pool as indexing) and dedupes chunk-level results down to the single best-scoring chunk per file, so one heavily-matched file doesn't crowd out the rest of the results.

### AI Answers (optional, Ollama)

`src-tauri/src/ollama.rs` proxies to a local Ollama instance (`localhost:11434`) via `reqwest` rather than calling it directly from the frontend via `fetch` - Ollama's CORS allowlist may not include the Tauri webview's origin, so proxying through Rust sidesteps that entirely. `reqwest` has TLS explicitly disabled (`default-features = false`) since the target is always plain `http://localhost`. Streaming responses are pushed token-by-token via a Tauri event (`ollama-token`) rather than the command's return value; a new request cancels any prior one via a shared cancellation flag before starting.

`OllamaProvider` centralizes availability/model-list state (checked on mount and re-checked whenever Settings opens, plus a manual refresh button) so Search and Settings see consistent state without duplicating the check. Answers are built from a RAG-style prompt (`src/lib/answer.ts`) over the top 5 search results' *full* chunk text (not the truncated display snippet), instructing the model to cite excerpt numbers; those citations render as clickable file-reveal chips (`src/lib/citations.ts`) that disambiguate by parent folder when two cited files share a filename.

### Permissions

Custom `#[tauri::command]`s (everything in `ingest.rs`, `store.rs`, `watcher.rs`, `hybrid_search.rs`, `ollama.rs`) are plain Rust functions and are **not** capability-gated. Only official Tauri plugin commands need entries in `src-tauri/capabilities/default.json` - currently `dialog:default`, `store:default`, `opener:default`, and `core:default` (which already expands to include `core:path:default`, covering `homeDir()` and friends - confirmed via the generated ACL manifest rather than assumed).

## Non-obvious things worth knowing

- **`pdf-extract` panics, not errors, on many malformed-but-common real-world PDFs** (bad Type3 font widths, unexpected encodings, etc.) - every call into it is wrapped in `std::panic::catch_unwind`, and extraction happens **page-by-page** (`output_doc_page`, the same function the crate's own per-page API uses) rather than whole-document, so one bad page doesn't lose an entire document's text.
- **`pnpm-workspace.yaml` has an `allowBuilds` gate** (a supply-chain policy specific to this environment) requiring explicit `true`/`false` per dependency wanting to run install scripts. `onnxruntime-node`, `protobufjs`, and `sharp` are set to `false` - they're transitive deps of `@huggingface/transformers` for its Node-native backend, which this app never uses (everything runs in the webview via WASM).
- The tray icon reuses the app's full-color icon rather than a proper macOS "template" (monochrome) icon - cosmetic only, not a bug.
- `package.json` pins `packageManager: "pnpm@11.18.0"`; use that major version to match the committed lockfile.

## Writing style guidelines

- **Avoid em dashes** - use regular hyphens (-) instead. Em dashes (—) can look like AI-generated text and reduce code readability. Use hyphens or restructure sentences to avoid them.

## Keeping this file current

When a change introduces a new subsystem, reverses or refines a decision documented above, or fixes a bug whose root cause is non-obvious from the code alone, update the relevant section (or add one) as part of that change - don't leave it for a separate pass. This file is read automatically at the start of every session in this repo, so it's the only place notes actually reach a future session without the user having to repeat context.
