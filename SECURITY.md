# Security Policy

OmniSearch-Local is a local-only app - it indexes files on your own machine and doesn't send file content anywhere, except an optional local Ollama connection (`localhost:11434`) that never leaves your device either. Still, if you find a security issue (e.g. a way to escape the sandboxed Tauri commands, a path traversal in indexing/watching, or anything that could let a malicious file or folder name execute code), please report it privately rather than opening a public issue.

## Reporting a vulnerability

Preferred: use [GitHub's private vulnerability reporting](https://github.com/wireframe-sensei/omnisearch-local/security/advisories/new) for this repo (Security tab → "Report a vulnerability").

Alternatively, email ubbarayaswanthreddy@gmail.com with details and, if possible, steps to reproduce.

Please don't disclose the issue publicly until a fix has shipped.

## What to expect

- Acknowledgement of your report as soon as possible.
- A fix developed privately, then a GitHub Security Advisory published alongside the patched release.

## Supported versions

This project is pre-1.0 and moving quickly - only the latest release is supported. Please make sure you're on the newest version before reporting.
