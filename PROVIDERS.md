# Burnrate provider specification

This document is the implementation contract for the Rust provider adapters. The endpoint and
credential details below were extracted from the shallow CodexBar reference checkout and checked
against the current Windows machine. Burnrate never imports browser cookies automatically. A manual
Cookie header is an explicit Settings fallback only.

## Credential paths verified on this Windows machine

| Provider | CLI / credential source | Machine result |
| --- | --- | --- |
| Claude | `claude auth login`; `%USERPROFILE%\\.claude\\.credentials.json` (or `CLAUDE_CONFIG_DIR\\.credentials.json`) | `C:\\Users\\Big G\\.claude\\.credentials.json` exists and contains `claudeAiOauth.accessToken`, `refreshToken`, `expiresAt`, `scopes`, `subscriptionType`, and `rateLimitTier`. |
| Codex | `codex login`; `%USERPROFILE%\\.codex\\auth.json` (or `CODEX_HOME\\auth.json`) | `C:\\Users\\Big G\\.codex\\auth.json` exists and contains `tokens.access_token`, `refresh_token`, `id_token`, `account_id`, and `last_refresh`. |
| Grok | `grok login`; `%USERPROFILE%\\.grok\\auth.json` (or `GROK_HOME\\auth.json`) | `C:\\Users\\Big G\\.grok\\auth.json` exists. It is a map keyed by an `https://auth.x.ai::...` scope; the selected entry contains `key`, `refresh_token`, `expires_at`, `auth_mode`, `email`, `team_id`, and identity fields. |
| Cursor | `cursor-agent login` / Cursor Agent session | `cursor-agent` is not installed on this machine. CodexBar's documented local-app fallback is a VS Code-style `globalStorage` SQLite DB with `ItemTable` key `cursorAuth/accessToken`; the Windows equivalent is `%APPDATA%\\Cursor\\User\\globalStorage\\state.vscdb` (not present with a usable token here). |
| OpenCode | `opencode auth login` (CLI auth) | No standalone auth file was found. The machine has `%USERPROFILE%\\.local\\share\\opencode\\opencode.db` (plus WAL/SHM), which is the local fallback database path. `%USERPROFILE%\\.config\\opencode\\opencode.jsonc` exists but has no credentials. |

## Common mapping

Every adapter maps provider data to the only UI shape:

```text
UsageSnapshot {
  provider, profile_name, plan_name?,
  windows: [{ label: "5h" | "Weekly" | "Monthly" | "Credits", used_pct, resets_at? }],
  fetched_at, status: "fresh" | "stale" | "error"
}
```

Percentages are clamped to 0..100. Unix reset seconds are converted to RFC3339. Missing optional
windows are omitted; a provider error keeps the last successful snapshot and marks it stale.

## Claude

* Auth source: `C:\\Users\\Big G\\.claude\\.credentials.json` (`claudeAiOauth.accessToken`).
  `CLAUDE_CONFIG_DIR` and `CLAUDE_SECURESTORAGE_CONFIG_DIR` override the root exactly as Claude Code
  does. `accessToken` is a bearer token; `expiresAt` is epoch milliseconds.
* Usage request: `GET https://api.anthropic.com/api/oauth/usage`.
  Headers: `Authorization: Bearer <accessToken>`, `Accept: application/json`,
  `Content-Type: application/json`, `anthropic-beta: oauth-2025-04-20`, and
  `User-Agent: claude-code/<detected version>`.
* Profile request (optional identity): `GET https://api.anthropic.com/api/oauth/profile` with
  `Authorization`, `Accept`, and `Content-Type`.
* Refresh: when `expiresAt` is expired, `POST https://platform.claude.com/v1/oauth/token` with
  `Content-Type: application/x-www-form-urlencoded` and `Accept: application/json`; body fields
  `grant_type=refresh_token`, `refresh_token=<refreshToken>`, and the public Claude Code OAuth
  `client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e`. Response fields are `access_token`, optional
  `refresh_token`, and `expires_in` seconds. Persist a refreshed current CLI login back to its
  credential file; persist refreshed saved profiles to their OS-keyring entries.
* Response fields: primary `five_hour.utilization` / `five_hour.resets_at`; weekly
  `seven_day.utilization` / `seven_day.resets_at`; optional `limits[]` entries (`kind`, `group`,
  `percent`, `resets_at`, `scope.model.display_name`, `is_active`); optional `extra_usage`.
  `five_hour` maps to `5h`, `seven_day` maps to `Weekly`, and an active weekly scoped limit can be
  added as another labeled window.
* Fallback chain: OAuth credentials file -> refresh endpoint -> Claude CLI probe (`claude` with
  `/usage`) -> no-login state. Browser/Claude web cookie fetch is deliberately excluded.

## Codex

* Auth source: `C:\\Users\\Big G\\.codex\\auth.json` (`CODEX_HOME` override). OAuth fields are
  `tokens.access_token`, `tokens.refresh_token`, optional `tokens.id_token`, and
  `tokens.account_id`.
* Usage request: `GET https://chatgpt.com/backend-api/wham/usage` (if a configured base URL does
  not contain `/backend-api`, use `GET <base>/api/codex/usage`). Headers:
  `Authorization: Bearer <access_token>`, `User-Agent: CodexBar`, `Accept: application/json`, and
  `ChatGPT-Account-Id: <account_id>` when present.
* Refresh: `POST https://auth.openai.com/oauth/token`, `Content-Type: application/json`, body
  `{ "client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "grant_type": "refresh_token",
  "refresh_token": "<refresh_token>", "scope": "openid profile email" }`. Read
  `access_token`, optional `refresh_token` and `id_token`; update `last_refresh`. If refresh fails,
  invoke the CLI fallback instead of logging the token.
* Optional credit request: `GET https://chatgpt.com/backend-api/wham/rate-limit-reset-credits` with
  the bearer, `Accept`, `User-Agent`, `OpenAI-Beta: codex-1`, `originator: Codex Desktop`, and
  `ChatGPT-Account-ID`.
* Response fields: `plan_type`; `rate_limit.primary_window.used_percent`, `reset_at`,
  `limit_window_seconds`; `rate_limit.secondary_window` with the same fields; optional
  `additional_rate_limits[].limit_name` and nested `rate_limit`; optional `credits`.
  Primary maps to `5h` when `limit_window_seconds` is five hours, otherwise the provider's current
  primary label; secondary maps to `Weekly`.
* Fallback chain: OAuth `auth.json` -> OAuth refresh -> `codex app-server` JSON-RPC
  (`account/read`, then `account/rateLimits/read`) -> NotLoggedIn/error. Never launch the Codex TUI
  for polling.

## Grok

* Auth source: `C:\\Users\\Big G\\.grok\\auth.json` (`GROK_HOME` override). Select a non-empty
  `https://auth.x.ai::...` entry first, then the legacy `https://accounts.x.ai/sign-in` entry.
  Read `key`, `refresh_token`, `expires_at`, `auth_mode`, `email`, `team_id`, `user_id`, and name
  fields. Grok CLI owns refresh; Burnrate only reads the cached credential.
* Primary CLI request: spawn `grok agent stdio`, send newline-delimited JSON-RPC 2.0
  `initialize` with `protocolVersion: "1"` and `clientCapabilities`, then `x.ai/billing` with an
  empty params object. The current installed `grok 1.0.5` returned `Method not found` for
  `x.ai/billing`; keep this route as the first capability probe for newer CLI versions.
* Bearer fallback: `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits` with
  `Authorization: Bearer <key>`, `x-xai-token-auth: xai-grok-cli`, `Accept: application/json`, and
  `User-Agent: CodexBar`. Optional plan request: `GET https://cli-chat-proxy.grok.com/v1/settings`
  with the same headers; read `subscription_tier_display`.
* Billing response fields: `config.creditUsagePercent`; fallback percentage is
  `config.onDemandUsed.val / config.onDemandCap.val * 100`; reset is
  `config.currentPeriod.end` then `config.billingPeriodEnd`; plan is
  `config.subscriptionTier` or settings `subscription_tier_display`.
  `x.ai/billing` uses `monthlyLimit.val`, `usage.totalUsed.val`, and
  `billingCycle.billingPeriodEnd` to compute the same percentage.
* Fallback chain: `grok agent stdio` -> CLI proxy bearer API -> aggregate local
  `%USERPROFILE%\\.grok\\sessions\\**\\signals.json` (`totalTokensBeforeCompaction`,
  `contextTokensUsed`, `modelsUsed`) -> NotLoggedIn/error. Browser cookies and gRPC-WKE are not
  used automatically.

## Cursor

* Auth source: Cursor Agent session first. If a local Cursor desktop installation exists, read the
  read-only VS Code-style SQLite store `%APPDATA%\\Cursor\\User\\globalStorage\\state.vscdb`,
  `ItemTable` key `cursorAuth/accessToken`, only while the JWT is not within 60 seconds of expiry.
  No automatic browser-cookie extraction is permitted.
* Usage request: `GET https://cursor.com/api/usage-summary` with the session bearer/cookie selected
  by the CLI session. Identity probe: `GET https://cursor.com/api/auth/me`. Legacy fallback:
  `GET https://cursor.com/api/usage?user=<id>`.
* Headers: `Accept: application/json`, `User-Agent: Burnrate`, and the CLI session's
  `Authorization: Bearer ...` or explicit manual `Cookie: ...` header. Manual cookie entry is the
  only non-CLI fallback and is forwarded as entered after validation.
* Response fields: usage summary plan/included usage, on-demand usage, and billing-cycle end;
  legacy usage has request counts and limits. Normalize the included plan percentage and cycle end
  to `Monthly`.
* Fallback chain: cursor-agent CLI session -> local Cursor `state.vscdb` -> manually pasted Cookie
  header -> NotLoggedIn. This machine has no cursor-agent binary or usable local token, so the UI
  must show `cursor-agent login`.

## OpenCode

* Auth source: OpenCode CLI auth (the current CLI stores auth outside the checked-in config) and an
  optional manually configured API key. Machine-local fallback database:
  `C:\\Users\\Big G\\.local\\share\\opencode\\opencode.db` with `-wal`/`-shm` companions.
  Read it read-only; never copy credentials into fixtures.
* API request (when API key is configured): `GET https://opencode.ai/zen/go/v1/usage` with
  `Authorization: Bearer <OPENCODE_API_KEY>`, `Accept: application/json`, and `User-Agent: Burnrate`.
  Legacy web dashboard fallback uses `POST https://opencode.ai/_server` with the server-function
  payload and explicit manual `Cookie` header only.
* Response fields: `workspaceId`; `rollingUsage.usagePercent`, `rollingUsage.resetInSec`; optional
  `weeklyUsage.usagePercent`, `weeklyUsage.resetInSec`; local billing may expose `monthlyUsage`,
  `monthlyLimit`, and `balance` (fixed-point scale 1e8). Map rolling to `5h`, weekly to `Weekly`,
  and local/API monthly data to `Monthly`.
* Fallback chain: CLI auth/API key -> manual OpenCode Cookie header -> read-only local DB ->
  NotLoggedIn. Browser cookie import is intentionally disabled. This machine has no detected CLI auth,
  so the empty state is `opencode auth login`.

## Phase 0 evidence

The authenticated responses captured before implementation are in `fixtures/claude-usage.json`,
`fixtures/codex-usage.json`, `fixtures/grok-billing.json`, and the raw CLI probe in
`fixtures/grok-agent-rpc.ndjson`. They contain no access, refresh, bearer, or cookie tokens.
