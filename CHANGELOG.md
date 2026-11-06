# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versioning: [Semantic Versioning](https://semver.org/).

"Unreleased" collects finished work between releases. A release is renaming that
section to `## [X.Y.Z] — DATE` and tagging `vX.Y.Z`: the release notes are
built **from these files**, not written again (`scripts/release-notes.sh`).

The changelog is kept in three languages: `CHANGELOG.md` (English, the main one),
`CHANGELOG_ru.md` and `CHANGELOG_uz.md`. Every change edits all three in one
commit; the version section headings are identical.

The version is declared in four places — `package.json`,
`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and here;
`scripts/check-versions.sh` compares them.

## [0.1.1] — 2026-11-05

### Added

- **Logs are visible in Settings** (Reports): the path and an “Open folder” button for the app log, and a list of the AI assistant's actions across all hosts, refusals included.
- **The client is now open source** under the GPL-3.0-or-later license. The source, build instructions and the list of everything the app sends over the network are in the repository; the Darwaza name and logo are not part of the license.
- **Whole folders can be transferred in the SFTP screen — in both directions and straight between two servers.** Progress is by the total size of the folder, it can be cancelled, and on an error the message names the file where it stopped. Nothing is overwritten (a copy gets a free name such as `proj (2)`), symbolic links are not followed or copied, and a half-copied file is removed. Copying between servers streams through the app without touching your disk.
- **SFTP is a permanent sidebar item with its own screen.** Two panes, each showing this computer or any server from your list; open two servers side by side, browse, create folders, rename, delete (with confirmation) and copy files between this computer and a server by button or drag and drop. If the host already has an open terminal tab, its connection is used; otherwise a separate files-only connection is made (no shell on the server).
- **The sidebar header shows the real app version** instead of a fixed "v0.1".
- **The assistant (agent access) works on Windows.** The app and `darwaza-mcp` talk over a named pipe whose name is derived from the token, so nobody can take the name before the app and get the token. The token is still checked in every request.
- **Space on a focused host row opens editing** (the pencil), Enter still connects; the row hint says so.

### Changed

- **A bare Ctrl+V in the terminal pastes nothing:** only `^V` is sent to the shell; paste with Ctrl+Shift+V or Shift+Insert.
- **The names the app stores your passwords under have changed** (the system keyring entry and the master-password check string now say Darwaza, not the old working name). Existing entries are carried over on their own the first time they are read or the master password is entered; nothing needs to be re-typed.
- **Larger text in the SFTP file lists and path bar,** and the right pane no longer runs past the window edge when a path or file name is long.
- **"Open previous tabs on start" is now off by default.** The app starts clean, without tabs; with the setting on it behaves as before. Tabs saved earlier are not erased — they come back if you turn it on.
- **The AI assistant settings in the host form are one collapsible section at the bottom, "For the AI assistant", collapsed by default.** Access, the risk warnings, read-only sudo commands and the assistant's notes live there; the notes block appears only when the assistant has written something. A collapsed section keeps its values, and a mark in its header shows when access is on or notes exist.

### Fixed

- **Windows build:** login through ssh-agent and agent forwarding no longer depend on a unix socket; the OpenSSH agent (`\\.\pipe\openssh-ssh-agent` or `SSH_AUTH_SOCK` if it holds a pipe path) is used.
- **The "ssh-agent not reachable" message no longer names a Unix-only variable.** On Windows the agent lives in the OpenSSH named pipe, not in `SSH_AUTH_SOCK`; the hint now just says to check that your system's ssh agent is running.
- **Saved passwords of unused accounts are moved to the new keyring name too.** After the rename, a password was moved only when something read it; an account with no hosts kept its entry under the old name for good. Now one pass at the first launch moves them all, and repeats on the next launch if the keyring was unavailable.
- **The agent plan confirmation window is fully in your language.** The reason a step is blocked, the step kind labels (check, edit, service…) and the "blocked / skipped" counters used to be in Russian or English whatever the interface language.
- **The caption at the bottom of the sidebar is no longer cut off.** "Local mode · sync off" ran past the edge in the Russian interface and now wraps.
- **Agent forwarding no longer hangs `ssh` and `git` on the server.** If your computer has no agent (or `SSH_AUTH_SOCK` points to nothing), Darwaza does not ask for forwarding and closes an already opened channel at once — before, the remote command waited for an answer that never came, until a timeout.
- **Ctrl+Shift+V and Shift+Insert paste text copied in other apps on Linux (Wayland).** Before, only text copied inside Darwaza itself was pasted and a foreign one was silently ignored; the clipboard is now read by the app itself, and a read error goes to the log instead of vanishing.
- **Spaces around a host address, user name and title no longer break the connection.** A host saved as ` 1.1.1.1` tried to connect to `root@ 1.1.1.1` and failed. Spaces at the edges are now trimmed on save and on connect, for hosts already saved or imported too; an empty value or a space inside the address is reported as an error.
- **Editing an existing host no longer asks for the password again.** Changing the group or the title meant typing the password anew; an empty field now means "keep the saved one", and the password is required only when creating a host.

### Security

- **On Windows the agent pipe is closed to other users and the token stays on the machine.** The pipe is created for the current user only, `darwaza-mcp` sends the token only after checking that the pipe was created by your own process, and the token file lives in `%LOCALAPPDATA%` (not the roaming profile) with access for you alone.

## [0.1.0] — 2026-11-01

The first public release of Darwaza — an SSH client that runs entirely on your
computer. An account and sync between devices are optional; the server cannot
read your data, it is encrypted on the device.

### Highlights

- **Works without an account and without a network.** Not a "free tier" but a
  principle: hosts, keys, terminal, SFTP and port forwarding work locally, and a
  sync error blocks nothing.
- **Hosts and groups.** A table with group, key and probe status, search and
  filters. A host connects with a double click or Enter. Import of
  `~/.ssh/config` and `known_hosts`; conflicting keys are shown separately.
- **Terminal with tabs and panes.** Up to four panes in any layout:
  Ctrl+Shift+D splits to the right, Ctrl+Shift+E downwards, dividers are dragged
  with the mouse, and a tab can be dropped on the edge of the terminal. Tab
  context menu: duplicate, rename, split, reconnect, close others.
  Alt+arrows switch tabs and panes; buffer search, output bookmarks, session
  recording in asciinema format, command palette (Ctrl+K), six terminal themes.
- **Copy and paste without surprises.** Ctrl+Shift+V and Shift+Insert paste the
  text whole and exactly once — a multi-line chunk is not executed line by line
  before you have read it. Ctrl+Shift+C and Ctrl+Insert copy the selection.
  Hotkeys work with a Cyrillic keyboard layout too.
- **Tabs come back after a restart** — one by one, so that password prompts do
  not pour in all at once. Can be turned off in settings.
- **Connecting: password, key, SSH agent, ProxyJump, agent forwarding,
  interactive authentication.** The host key is checked on the first connection;
  a mismatch gets a separate warning.
- **If it did not connect, you are told why:** the server refused the connection,
  did not accept the credentials, the host name was not found — in the interface
  language, without raw engine strings.
- **SFTP** — two panes, navigation, streamed file transfer. The "Files" button in
  the terminal sidebar opens the files of the current session.
- **Port forwarding** — local, remote and dynamic (SOCKS5), with autostart on
  connect.
- **Snippets** with substitution of `{{host}}`, `{{user}}`, `{{port}}`, `{{name}}`.
- **Secrets in the system keyring** (KWallet, GNOME Keyring); no passwords in the
  database. A new settings section shows where exactly they are kept. If there
  is no keyring, the app warns you before you type a password and offers a
  master password or entering it on connect.
- **Master password** (Argon2id + AES-256-GCM) and a second factor (TOTP) on top
  of it.
- **Account and sync between devices** — optional. A forgotten password can be
  recovered with a key on paper: 32 characters issued at registration. There is
  no reset by e-mail and never will be — the server cannot read your copy; that
  is how the system is built, not a shortcoming.
- **Update check** — only on your button in "About"; the app never goes to the
  network on its own. A downloaded update is verified by signature.
- **Six interface languages:** uz, ru, en, kk, ky, tr. Light and dark themes,
  the Darwaza palette, fonts bundled with the app.
- **Crash reports to disk.** Collection is off by default; the app sends nothing
  anywhere.

### AI assistant (agent access)

The assistant works only with hosts where you have enabled access for it, and
does nothing without your confirmation except reading.

- **A plan with confirmation.** You see how the assistant introduced itself,
  which session it comes from, the server address, the user and the time left
  before the automatic refusal. Steps are labelled by nature (check, service,
  file edit, command), file edits are shown as a line-by-line diff. The safe
  action — reject — has the focus.
- **A forbidden step is shown but never executed.** It is struck through with the
  reason, and execution stops before it.
- **The plan result without false success:** how many steps were done, where the
  error was, what is forbidden, what did not run. There is no command output in
  the result — it may contain passwords. An expired plan is marked and cannot be
  confirmed.
- **The app calls you when your decision is needed:** the taskbar icon lights up
  and a notification arrives. It contains neither commands nor host names. On KDE
  Plasma (Wayland) the notification is critical and does not close by itself.
  Can be turned off in settings.
- **What the assistant can do:** a server summary in one call, file search,
  reading logs while they run, editing large configs by describing the change,
  safe service restarts (configuration check before and state check after), work
  with files on the server, copying between servers directly, access to internal
  services through a tunnel that closes by itself, your snippets (behind a
  separate switch that is off by default), a note about the server and a journal
  of earlier actions.
- **Individual sudo commands** can be allowed to the assistant as a list on a
  specific host. Without the list nothing is allowed.
- **A tab with a typed command.** For what the assistant may not do itself, it
  types the command into the terminal, and you press Enter.

### Security

- **A master password shorter than 12 characters is not accepted** — the check is
  in the core, not only in the window.
- **Session recordings and the database are closed to other users of the
  computer.** A recording holds the whole terminal output — configs, tokens,
  echoed passwords. The data and recordings directories are `0700`, files are
  `0600`.
- **Changing the account password does not risk synced data.** Records are
  encrypted with a separate key that the password only unlocks.
- **A step of a confirmed plan is not cut off midway** (up to 15 minutes), so the
  server is not left half-changed; a read-only command is cut off after 30
  seconds.
- **Enabling the assistant's access to a host needs a separate confirmation;**
  disabling happens at once.

### Foundation

Where it all started (the first working version, May 2026):

- Hosts and groups: create, edit, delete, search, import of `~/.ssh/config`.
- SSH connection with a password or a private key, a server PTY, several
  simultaneous sessions, keep-alive.
- Terminal on xterm.js: tabs, buffer search, copying, links, font settings.
- Known hosts by TOFU: confirmation on the first connection, a separate warning
  on a key mismatch, a management page.
- Secrets in the system keyring — no passwords in the database.
- Interface languages: uz, ru, en.

<!-- Comparison and release links will appear here once the project address is
     known. Inventing one beforehand is wrong: a wrong link in release notes is
     worse than none — people follow it and do not come back. -->
