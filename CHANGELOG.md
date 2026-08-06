# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-06

Initial public release.

### Added
- Spotlight-style floating search bar, summoned with a global hotkey.
- Two-phase indexing pipeline: instant keyword search via BM25/FTS5, followed by background semantic embedding.
- Hybrid search (keyword + semantic) blended via Reciprocal Rank Fusion.
- Live file watching for automatic incremental re-indexing.
- Broad file format support: text/code, HTML, RTF, PDF, DOCX, PPTX, XLSX.
- Optional AI answers via a local Ollama instance, with citation chips linking back to source files.
- Cross-platform installers for macOS, Windows, and Linux.

[Unreleased]: https://github.com/wireframe-sensei/omnisearch-local/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/wireframe-sensei/omnisearch-local/releases/tag/v0.1.0
