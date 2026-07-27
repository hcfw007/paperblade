# PaperBlade

[![CI](https://github.com/hcfw007/paperblade/actions/workflows/ci.yml/badge.svg)](https://github.com/hcfw007/paperblade/actions/workflows/ci.yml)

A real desktop PDF toolkit. All the power. None of the docker-compose.

Your files never leave this computer. No uploads, no accounts, no network calls
in any of the core features.

## Tools

| Tool | What it does | Status |
|------|--------------|--------|
| **Merge** | Combine several PDFs into one, in the order you choose | Working |
| **Split** | By page ranges (`1-3, 5, 8-10`) or into fixed-size chunks | Working |
| **Encrypt & Decrypt** | Add or remove a password (AES-128) | Working |
| **Compress** | Quality presets — 75% off a scanned page, 13% off text | Working¹ |
| Watermark | Stamp text or an image on every page | Planned |
| Convert | PDF to images, images to PDF | Planned |

¹ Compress needs Ghostscript. This build doesn't bundle it yet — it falls back
to whatever is on your `PATH`, and the page tells you when it can't find one.
`brew install ghostscript` covers it for now. Everything else is self-contained.

## Status

Early WIP, macOS first. The three shipped tools work end to end and are covered
by tests — including verification against macOS PDFKit, rather than only reading
our own output back. Not yet signed, notarized, or released as a build you can
double-click; that's Milestone 3. See [ROADMAP.md](docs/ROADMAP.md).

## Stack

- [Tauri 2](https://tauri.app) — Rust shell, small binary
- [SvelteKit 5](https://svelte.dev) — SPA frontend, no server
- [lopdf](https://github.com/J-F-Liu/lopdf) — pure-Rust PDF library, embedded;
  handles every structural operation with no external process
- [Ghostscript](https://www.ghostscript.com) — external process, compression
  only. Not bundled yet; see the note under Tools and the License section.

[ARCHITECTURE.md](docs/ARCHITECTURE.md) covers how these fit together.

## Develop

Requires Rust, Node 22+, pnpm, and Xcode Command Line Tools.

```sh
pnpm install
pnpm tauri dev
```

`pnpm dev` runs the frontend alone in a browser — fine for UI work, but the Rust
commands are unavailable there.

## Testing

CI runs all of these on every pull request.

```sh
cd src-tauri
cargo test                                  # PDF logic
cargo clippy --all-targets -- -D warnings
cargo fmt --check

cd ..
pnpm check                                  # svelte-check
pnpm build
```

## License

MIT, for everything in this repository.

Compression shells out to **Ghostscript**, which is **AGPL v3** — a different
and much stickier licence. Today that's your own copy on `PATH`, invoked as a
separate process, so nothing AGPL is distributed here and this stays a plain MIT
project.

That changes the day a build ships Ghostscript inside it. Doing so means
attributing Ghostscript, carrying its licence, and offering its corresponding
source. It also rules out closed-source distribution and the Mac App Store
without a commercial licence from [Artifex](https://artifex.com/licensing/).
Worth knowing before packaging work starts.
