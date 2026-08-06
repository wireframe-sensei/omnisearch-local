<div align="center">

# OmniSearch-Local

**Find anything on your computer just by describing it - entirely offline.**

No cloud. No telemetry. No API keys. Your files never leave your machine.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)](#prerequisites)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%20v2-24C8DB)](https://tauri.app)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

<!-- TODO: add a screenshot or a short demo GIF of the search bar in action here - this is the single highest-impact thing you can add to this README. -->

</div>

---

## Why OmniSearch-Local?

Most "AI search" tools ship your files to someone else's server. OmniSearch-Local doesn't:

| | Cloud search tools | OmniSearch-Local |
|---|---|---|
| File content leaves your device | ✅ Usually | ❌ Never |
| Requires an API key / subscription | ✅ Often | ❌ No |
| Works with no internet connection | ❌ Rarely | ✅ Yes (after first-run model download) |
| Understands natural-language queries | ✅ Yes | ✅ Yes |
| Open source | ❌ Rarely | ✅ MIT licensed |

Type a description of what you're looking for - *"notes about the budget meeting last quarter"* - and get back ranked matches with highlighted snippets, even if none of your exact words appear in the file. Ask a local LLM to synthesize an answer from your top matches instead of just listing files, if you want.

## Features

- 🔍 **Spotlight-style search bar** - a minimal, floating window summoned with a global hotkey (`Option+Shift+Space`), just like Spotlight or Raycast. Runs as a background/tray utility with no persistent Dock or taskbar icon.
- 🧠 **Hybrid semantic + keyword search** - local embedding similarity (via [`transformers.js`](https://huggingface.co/docs/transformers.js), running in-browser with no network calls after the first model download) blended with BM25 keyword matching via Reciprocal Rank Fusion, so exact terms (filenames, error codes) and conceptual matches both surface well.
- ⌨️ **Keyboard-first** - arrow keys + Enter to browse and open results, no mouse required.
- 👀 **Live indexing** - pick folders in Settings; a file watcher picks up creates/edits/deletes automatically and re-indexes incrementally (unchanged files are skipped).
- 📄 **Broad file support** - `.txt`, `.md`, `.csv`, `.log`, most code/config files, `.html`/`.htm`, `.rtf`, `.pdf`, `.docx`, `.xlsx`, `.pptx`.
- 🤖 **Optional local AI answers** - if [Ollama](https://ollama.com) is installed and running, ask a question and get a streamed, cited answer synthesized from your search results, with a model picker in Settings.
- 📂 **Reveal in Finder/Explorer** - clicking a result or an answer's citation opens its containing folder with the file selected.

## Tech stack

- **App shell:** [Tauri v2](https://tauri.app) (Rust backend + native OS webview - a fraction of Electron's footprint)
- **Frontend:** React 19, TypeScript, Vite, Tailwind CSS v4, shadcn-style components
- **Local embeddings:** [`@huggingface/transformers`](https://huggingface.co/docs/transformers.js) (`Xenova/all-MiniLM-L6-v2`, WASM, runs in the webview)
- **Storage:** SQLite via `rusqlite` (bundled, zero system dependencies)
- **Search:** BM25 implemented in Rust + Reciprocal Rank Fusion with embedding similarity
- **File parsing:** `pdf-extract`, `html2text`, `rtf-parser`, `calamine` (xlsx), `zip`/`quick-xml` (docx/pptx)
- **Optional LLM:** [Ollama](https://ollama.com)'s local HTTP API, streamed via `reqwest`

## Prerequisites

- [Node.js](https://nodejs.org/) (v20+) and [pnpm](https://pnpm.io/)
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain, via `rustup`)
- Tauri's platform-specific dependencies - follow the [Tauri prerequisites guide](https://tauri.app/start/prerequisites/) for your OS (Xcode Command Line Tools on macOS, WebView2 on Windows, standard build tools + WebKitGTK on Linux)
- Optional, for AI answers: [Ollama](https://ollama.com) installed and running locally, with at least one model pulled (e.g. `ollama pull llama3.2`)

## Getting started

```bash
# Clone and install dependencies
git clone https://github.com/wireframe-sensei/omnisearch-local.git
cd omnisearch-local
pnpm install

# Run in development mode (opens the app with hot reload)
pnpm tauri dev
```

The first search will download the local embedding model (~30MB) from Hugging Face - a one-time fetch, cached afterward for fully offline use. If you have Ollama running, AI answers work automatically; otherwise that feature just stays hidden.

## Using the app

1. **Add folders to index:** click the gear icon (or open Settings) and add one or more directories.
2. **Search:** press `Option+Shift+Space` from anywhere to summon the search bar, then type a plain-English query.
3. **Navigate:** use arrow keys + Enter, or click a result, to reveal the file in Finder/Explorer.
4. **Ask AI (optional):** with results showing, press `Cmd+Enter`/`Ctrl+Enter` or click "Ask AI about these results" to get a synthesized answer (requires Ollama).
5. **Quit:** closing the window or `Cmd+Q` hides the app to the tray instead of quitting - use the tray icon's "Quit" to fully exit.

## Building for production

```bash
pnpm tauri build
```

This produces a native installer/bundle for your platform under `src-tauri/target/release/bundle/`.

## Running tests

The Rust backend has unit test coverage for scanning, extraction, storage, and search ranking:

```bash
cd src-tauri
cargo test
cargo clippy --all-targets
```

## Roadmap

- [ ] Image support via OCR (opt-in, since it's slower to index)
- [ ] Configurable global hotkey (avoid conflicts like 1Password's default binding)
- [ ] `.xls`/`.ods` legacy spreadsheet formats
- [ ] Exclude-pattern support for indexed directories (skip specific subfolders)
- [ ] Recent-searches list

Have an idea? [Open an issue](../../issues) - see [Contributing](#contributing) below.

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for how to set up a dev environment, coding conventions, and the PR process. Please also read the [Code of Conduct](CODE_OF_CONDUCT.md).

Found a security issue? Please see [SECURITY.md](SECURITY.md) instead of opening a public issue.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release history.

## License

MIT - see [LICENSE](LICENSE) for details.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Acknowledgments

Built on [Tauri](https://tauri.app), [Hugging Face Transformers.js](https://huggingface.co/docs/transformers.js), [Ollama](https://ollama.com), and a handful of excellent Rust crates - see [`src-tauri/Cargo.toml`](src-tauri/Cargo.toml) for the full list.

App icon from [SVG Repo](https://www.svgrepo.com).

---

<div align="center">

If this project is useful to you, consider giving it a ⭐ - it helps others find it too.

</div>
