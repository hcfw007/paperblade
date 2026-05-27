# PaperBlade

A real desktop PDF toolkit. All the power. None of the docker-compose.

Your files never leave this computer.

## Status

Early WIP. macOS first.

## Stack

- [Tauri 2](https://tauri.app) — Rust shell, tiny binary
- [SvelteKit 5](https://svelte.dev) — SPA frontend
- [qpdf](https://github.com/qpdf/qpdf) / [Ghostscript](https://www.ghostscript.com) — bundled CLI engines

## Develop

Requires Rust, Node 22+, pnpm, and Xcode Command Line Tools.

```sh
pnpm install
pnpm tauri dev
```

## License

MIT
