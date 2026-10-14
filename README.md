# Cumulas Mail

A fast, native-feeling Outlook alternative for Linux: mail, calendar, contacts
and tasks for **Microsoft 365 / Outlook.com** (via Microsoft Graph) and any
**IMAP/SMTP** account.

Built with [Tauri 2](https://tauri.app) (Rust) and React. Targets Flathub.

> Status: early development.

## Development

Requirements: Rust (stable), Node.js 22, and the Tauri system libraries:

```sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libayatana-appindicator3-dev librsvg2-dev libssl-dev libdbus-1-dev
```

```sh
npm install
npm run tauri dev     # desktop app with hot reload
npm run dev           # UI only, in the browser, with demo data (?empty for onboarding)
```

Checks (the same as CI):

```sh
npm run typecheck && npm run lint && npm test
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [docs/architecture.md](docs/architecture.md) for how the pieces fit
together.
