# Darwaza

A local-first SSH and SFTP client for Linux: host manager, tabbed terminal with
split panes, snippets, port forwarding, a two-pane SFTP screen, and an optional
AI-assistant channel where **you confirm every change** before it runs.

Everything works **on your computer, with no account and no network**. An account
and sync between devices are optional, and the server can never read your data:
it is encrypted on the device with a key derived from your master password.
That is a principle, not a missing feature — a forgotten password cannot be reset.

Built with Tauri 2 (Rust), Vue 3, xterm.js and [russh](https://github.com/Eugeny/russh).
Languages: Uzbek, Russian, English, Kazakh, Kyrgyz, Turkish.

## Download

Ready-made packages (`.deb`, `.rpm`, AppImage) are on the
[Releases](https://github.com/ikhasanmusaev/darwaza/releases) page.
Arch / Manjaro: `cd packaging/arch && makepkg -si`.

### Windows

Download the `Darwaza_<version>_x64-setup.exe` installer from the same page. It is
**not code-signed**, so Windows SmartScreen will say "Windows protected your PC":
click **More info → Run anyway**. Compare the file with `SHA256SUMS` from the
release if you want to be sure it is the one we published.

Your data (hosts, settings, session recordings) lives in
`%APPDATA%\darwaza\darwaza\data`; the AI-assistant access token is in
`%LOCALAPPDATA%\darwaza\darwaza\data`. Uninstalling the app **does not delete
them**, so a reinstall finds your hosts again. For a complete removal, delete both
folders by hand after uninstalling. Passwords and key passphrases are kept in Windows
Credential Manager (entries named `darwaza`); remove them there if you want them gone.

## SFTP is still rough

File transfer works, but it is not comfortable yet, and we know it. Next releases
will bring:

- asking what to do when a file with the same name exists: replace, skip or rename;
- deleting a folder with everything inside, after a confirmation;
- selecting several files at once;
- an editable path bar and sorting by name, size and date;
- changing permissions (chmod);
- editing a server file in your own editor;
- the file panel following the terminal's current folder;
- reconnecting after a dropped connection, folder bookmarks;
- transfer speed and time left.

If something gets in your way, open an issue: it helps us choose what to do first.

## Build from source

You need Rust (stable), Node.js 22, [pnpm](https://pnpm.io) and the system
libraries for Tauri 2 on Linux (WebKitGTK 4.1, libsoup 3, `librsvg`, `patchelf`;
see the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)).

```bash
pnpm install
pnpm tauri dev        # run in development mode (UI on http://localhost:1420)
pnpm tauri build      # the real, installable build: .deb, .rpm, AppImage
```

Build what you ship **only** with `pnpm tauri build` (`--no-bundle` is fine for a
bare binary). A plain `cargo build --release` produces the same app but pointed at
the dev server on `localhost:1420`, so it shows a white screen when no dev server
runs.

### Checks

```bash
pnpm lint
pnpm exec vue-tsc --noEmit
python3 scripts/check-locales.py            # all six languages carry the same keys
pnpm exec playwright test                   # UI tests; needs the dev server on :1420
cd src-tauri && cargo test --lib            # Rust unit tests
```

Tests that need a real SSH server are marked `#[ignore]`; `scripts/live-sshd.sh`
starts a throwaway `sshd` on `127.0.0.1:2222` without root.

## What the client sends over the network

Nothing happens in the background. The client opens a network connection only in
these cases:

| When | Where to | Code |
|---|---|---|
| You connect to a host, open SFTP, or use port forwarding | the SSH servers you configured (and their jump hosts) | `src-tauri/src/commands/ssh.rs`, `sftp.rs`, `forwarding/` |
| The host list probes reachability | the hosts you saved, TCP port only | `src-tauri/src/commands/probe.rs` |
| You press **Check for updates** in *About* | `https://darwaza.uz/updates/…`; the download is verified by an Ed25519 signature | `tauri-plugin-updater`, `src-tauri/tauri.conf.json` |
| You sign in to an account (optional) | `https://api.darwaza.uz` or the server address you set | `src-tauri/src/sync/api.rs` |
| You press **Send** on a crash report and set an endpoint | the endpoint you typed; collection is off by default | `src-tauri/src/commands/crash.rs` |

Without an account, no update check and no crash endpoint, the only traffic is to
your own servers. To see it yourself, run the app under a connection monitor, for
example `strace -f -e trace=connect ./darwaza`, or search the sources for the
HTTP client: `grep -rn "reqwest" src-tauri/src`.

The AI-assistant channel (`darwaza-mcp`) is a local Unix socket (a named pipe on
Windows) guarded by a random token; it is not a network listener.

## License

Darwaza is free software: you can redistribute it and modify it under the terms of
the GNU General Public License, version 3 or (at your option) any later version.
See [LICENSE](LICENSE). The bundled fonts (IBM Plex Sans, JetBrains Mono) are under
the SIL Open Font License; their texts are in `public/fonts/`.

## Name and logo

The **Darwaza** name and the logo are not part of the GPL grant: the license covers
the code, not the brand. If you distribute a modified version, please give it a
different name and your own logo so that people can tell it from the original.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md). To report a vulnerability, follow
[SECURITY.md](SECURITY.md) — please do not open a public issue for it.

*A short Russian version is in [README_ru.md](README_ru.md).*
