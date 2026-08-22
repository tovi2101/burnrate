# Burnrate

Burnrate is a small, open-source system-tray popover for seeing Claude, Codex, Grok, Cursor, and
OpenCode usage limits together. It is a Tauri v2 app with a Rust backend and React + TypeScript +
Tailwind frontend. There is no account server, telemetry, analytics, or Electron layer.

## Install

Burnrate is directly logged in and synced from your existing provider CLI sessions on first
launch—there is zero setup.

### Windows

Install the current release with [Scoop](https://scoop.sh/):

```powershell
scoop install https://raw.githubusercontent.com/tovi2101/burnrate/master/packaging/scoop/burnrate.json
```

MSI and NSIS installers are also attached to each [GitHub Release](https://github.com/tovi2101/burnrate/releases).
The binaries are unsigned, so Windows SmartScreen may appear; click **More info**, then **Run
anyway**.

### macOS

The cask in `packaging/homebrew/burnrate.rb` is ready for the `tovi2101/homebrew-tap` tap:

```bash
brew install --cask tovi2101/tap/burnrate
```

The macOS build is universal (Apple Silicon and Intel). Because it is unsigned, Gatekeeper may
block its first launch. Right-click Burnrate and choose **Open**, or run:

```bash
xattr -dr com.apple.quarantine /Applications/Burnrate.app
```

### Linux

Install the latest AppImage into `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/tovi2101/burnrate/master/packaging/install.sh | bash
```

The same release also includes a Debian package.

## Develop

Install Node.js, Rust, and the [Tauri v2 platform prerequisites](https://v2.tauri.app/start/prerequisites/), then:

```bash
npm install
npm run tauri dev
```

For a frontend-only preview (the UI uses the checked-in real fixtures when Tauri is unavailable):

```bash
npm run dev
```

Build the web bundle with `npm run build`. The Windows development target is the same code path as
macOS; provider paths are resolved from `USERPROFILE` on Windows and `HOME` elsewhere.

## How detection works

On launch Burnrate looks for the provider CLI credential files described in [PROVIDERS.md](./PROVIDERS.md):

* Claude: `~/.claude/.credentials.json` (`claude auth login`)
* Codex: `~/.codex/auth.json` (`codex login`)
* Grok: `~/.grok/auth.json` (`grok login`)
* Cursor: a cursor-agent session or the local Cursor `state.vscdb` auth record
* OpenCode: CLI auth/API configuration, then its local database fallback

The app prefers each provider CLI's local RPC or authentication command, then reads credentials
fresh from its configured source for read-only usage requests. The exact request methods, headers,
response fields, and fallback order are the implementation spec in `PROVIDERS.md`.

Cursor and OpenCode are intentionally shown as a clear login empty state when no CLI credentials are
present. The exact commands are `cursor-agent login` and `opencode auth login`.

## Multiple accounts

Use Settings → Profiles → “save credential source label”. Burnrate stores a reference to the CLI
credential source and its account identity, never a frozen OAuth token. Labels that resolve to the
same source share one poll; switching from a card is instant, and “All” stacks the configured labels.
Claude, Codex, and Grok expose one active account per CLI config directory, so multiple labels on
one directory follow that same active login. Simultaneous accounts require separate CLI config
directories. Deleting a profile removes only Burnrate's source reference and index entry.

## Safety

Burnrate is a read-only observer: it reads credentials but never modifies your provider login state
or writes rotated tokens. OAuth refresh is delegated to each provider's own CLI so its credential
file remains authoritative and the coding agent stays logged in. Credential files are reread before
every poll, and profile labels that point to the same account are coalesced into one request.

## Privacy and security

Everything stays local. Quota snapshots are cached under the platform cache directory so the popover
can open with the last known state. The cache contains quota data only, never credentials. Failed
refreshes keep the last known values and mark them stale rather than blanking a card. Burnrate never
extracts browser cookies automatically; a manual Cookie header is the only web fallback and is an
explicit Settings action.

The app makes no network calls except to the configured provider endpoints. It never logs access
tokens, refresh tokens, cookies, or raw authorization headers.

## Screenshots

![Burnrate popover with live provider usage](screenshots/verified-popover.png)

![Settings and persisted provider preferences](screenshots/verified-settings-persist.png)

![Multiple provider profiles](screenshots/verified-profiles.png)

![Rendered tray icon at 4x](screenshots/verified-tray-icon.png)

## Attribution

Provider endpoint research is based on [CodexBar by Peter Steinberger](https://github.com/steipete/CodexBar),
MIT licensed. The checked-in `references/` directory is development research only and is excluded
from published builds.

Provider names and marks are trademarks of their respective owners and are used for identification
only. Burnrate is not affiliated with or endorsed by any provider.

## License

MIT. See [LICENSE](./LICENSE).
