# Usage history source map

Research snapshot: 2026-08-22. Reference inputs are T3 Code shallow commit `592c5983`, the local CodexBar shallow checkout, and ccusage shallow commit currently under `references/ccusage`. Counts and date coverage below were measured without printing transcript content or credentials.

## Rules of record

- A source is stored in its native unit. Transcript token counts become `tokens`; quota snapshots become `used_pct`. They are never converted into one another.
- Local CLI transcripts generally do not identify the account that produced an old turn. Those rows use profile `Local CLI`; they are not attributed to whichever profile happens to be selected today.
- Message identifiers are retained only as local deduplication keys. Prompts, responses, tool payloads, credentials, and raw transcripts are never copied into Burnrate's database.
- Provider endpoints described as current-only are not called for backfill. Existing polls append their current snapshots without adding a request.
- Browser cookies are never extracted automatically. Cursor history is available only when the user has supplied the same manual cookie fallback already supported by Burnrate.

## Source map

### Claude

**Local backfill — primary.** `~/.claude/projects/**/*.jsonl` (`C:\Users\Big G\.claude\projects\**\*.jsonl` here). The assistant record shape is `type`, `timestamp`, `sessionId`, `requestId`, and `message.{id,model,usage}`. Usage fields are `input_tokens`, `output_tokens`, `cache_creation_input_tokens`, and `cache_read_input_tokens`; optional `costUSD` is provider-reported API-equivalent cost. Repeated content blocks are deduplicated by the `message.id + requestId` pair, matching T3 Code and ccusage. Nested subagent files are included and the same global key prevents parent/subagent copies from being counted twice.

Measured here: 69 JSONL files, 1,722 unique usage records, 378,059,564 processed tokens, from `2026-08-21T00:52:12.053Z` through `2026-08-22T18:05:36.093Z` (2 UTC days). This machine currently has two days—not weeks—of surviving Claude transcript coverage. Burnrate must label the chart `history since Aug 21` rather than imply a full 30-day record.

**Historical account endpoint — unavailable to the subscription login.** CodexBar implements two Anthropic Admin API calls, both `GET` with `x-api-key`, `anthropic-version: 2023-06-01`, and daily range parameters `starting_at`, `ending_at`, `bucket_width=1d`, `limit=31`:

- `https://api.anthropic.com/v1/organizations/usage_report/messages?group_by[]=model`: daily `data[].{starting_at,ending_at,results[]}`, with result fields `uncached_input_tokens`, `cache_creation.{ephemeral_1h_input_tokens,ephemeral_5m_input_tokens}`, `cache_read_input_tokens`, `output_tokens`, and `model`.
- `https://api.anthropic.com/v1/organizations/cost_report?group_by[]=description`: daily `data[].{starting_at,ending_at,results[]}`, with result fields `currency`, `amount`, `description`, and `cost_type`.

These require an Admin API key, which Burnrate does not have and must not infer from the Claude Code OAuth login, so they are documented but not used. The subscription endpoint `GET https://api.anthropic.com/api/oauth/usage` returns only current `five_hour`, `seven_day`, scoped limits, resets, and extra-usage totals; it is not history.

**Live source.** Every existing Claude poll appends each returned window as `used_pct:<window label>` with its reset instant as the window-instance boundary. No extra request is made.

### Codex

**Local backfill — primary.** `~/.codex/sessions/**/*.jsonl` (`C:\Users\Big G\.codex\sessions\**\*.jsonl` here). `turn_context.payload.model` supplies the active model. `event_msg.payload.type == "token_count"` supplies `info.last_token_usage.{input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens}` and the record timestamp. `input_tokens` includes cached input; the stored processed total counts uncached, cached-read, cache-write, and output once. Consecutive identical usage payloads are dropped. Fork/subagent history copied in the opening one-second burst is suppressed, matching T3 Code/ccusage.

Measured here before fork-copy reconciliation: 4 JSONL files and 4,499 eligible deltas, spanning `2026-08-21T08:16:16.705Z` through `2026-08-22T21:18:53.159Z` (2 UTC days), with 603,000,832 raw processed tokens. The production backfill applies the stricter fork and duplicate rules, so its authoritative total may be lower.

**Provider endpoints.** CodexBar calls `GET https://chatgpt.com/backend-api/wham/usage` (fallback `/api/codex/usage`) with the OAuth bearer and account header. Its response is a current snapshot: `account_id`, `plan_type`, `rate_limit.{primary_window,secondary_window}`, each window containing `used_percent`, `reset_at`, and `limit_window_seconds`; it can also contain `credits`, `individual_limit`, `spend_control`, and `additional_rate_limits`. It has no past buckets. `GET https://chatgpt.com/backend-api/wham/rate-limit-reset-credits` and account spend-control monthly-usage calls also describe current credits/month totals, not time series.

**Live source.** Existing Codex polls append current percentages and reset boundaries only.

### Cursor

**Local backfill.** Neither T3 Code nor ccusage defines a Cursor CLI transcript source. No honest local token/day lane was found.

**Historical account endpoint — conditional primary.** CodexBar implements `POST https://cursor.com/api/dashboard/get-filtered-usage-events` with JSON body `{page,pageSize,startDate,endDate}`, `Cookie`, `Origin: https://cursor.com`, `Content-Type: application/json`, and `Accept: application/json`. Passing no dates requests all available account history; pages are 1,000 events and `totalUsageEventsCount` is the authoritative completion count. The response carries `usageEventsDisplay[]` with `timestamp` (Unix ms, often a string), `model`, `tokenUsage.{inputTokens,outputTokens,cacheWriteTokens,cacheReadTokens,totalCents}`, `kind`, `requestsCosts`, `usageBasedCosts`, `cursorTokenFee`, `chargedCents`, `isChargeable`, `isHeadless`, `owningUser`, and `owningTeam`. This yields genuine per-event token and spend history.

This machine has no Cursor login/manual cookie, so endpoint backfill is unavailable here. Burnrate may use it after a user supplies the existing manual cookie; it will never extract browser cookies. Current-only supporting calls are `GET /api/usage-summary`, `GET /api/auth/me`, and legacy `GET /api/usage?user=<id>`.

**Live source.** Existing Cursor polls append percentages only when configured.

### Grok

**Local backfill — primary.** Current Grok Build CLI history is `~/.grok/sessions/**/updates.jsonl`. Completed-turn records have `params.update.sessionUpdate == "turn_completed"`, `params._meta.{eventId,agentTimestampMs}`, `params.sessionId`, and `params.update.usage`. Usage fields are `inputTokens`, `outputTokens`, `cachedReadTokens`, `cacheCreationTokens`, `reasoningTokens`, `totalTokens`, `costUsdTicks`, and optional per-model `modelUsage`. Input includes cache; reasoning is already included in output. `costUsdTicks` is fixed-point USD at 1e-10 USD per tick. Event IDs deduplicate copies across sessions.

Measured here: 2 `updates.jsonl` files, 3 completed-turn usage records, 20,352,353 processed tokens, from `2026-08-20T23:55:09Z` through `2026-08-21T22:37:41Z` (2 UTC days).

The requested legacy/current aggregate lane `~/.grok/sessions/**/signals.json` also exists: 2 files here. Its observed fields include `contextTokensUsed`, `contextWindowTokens`, `contextWindowUsage`, `totalTokensBeforeCompaction`, `compactionCount`, message/turn/tool counts, model IDs, latency aggregates, and session duration. It has no per-turn timestamp or daily buckets. Burnrate parses it as an honest `session_context_tokens` snapshot using the file modification time only when no completed-turn history exists for that session; it is not presented as tokens/day and is never spread across earlier dates.

**Provider endpoints.** `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits` with the Grok bearer and `x-xai-token-auth: xai-grok-cli` returns current `config.{creditUsagePercent,currentPeriod.end,billingPeriodEnd,onDemandCap.val,onDemandUsed.val,subscriptionTier}`. The CLI RPC `x.ai/billing` exposes the same current billing state. Neither is a historical time series.

**Live source.** Existing Grok polls append the current credit/quota percentage and reset boundary only.

### OpenCode

**Local backfill — primary when populated.** The real Windows path is `C:\Users\Big G\.local\share\opencode\opencode.db`. The observed SQLite schema has a `message` table with `id`, `session_id`, `time_created`, `time_updated`, and JSON `data`; message JSON exposes `providerID`, `modelID`, `time.created`, `tokens.{input,output,total,cache.{read,write}}`, and optional `cost`. The `session` table also has cumulative `cost`, `tokens_input`, `tokens_output`, `tokens_reasoning`, `tokens_cache_read`, and `tokens_cache_write`, but those are not summed with messages because they would double count. Legacy installs may also have `storage/message/<session>/<message>.json`; database rows win by message ID.

The database exists on this machine, but currently contains 0 messages with usage, so its historical coverage is empty. The parser is still implemented because this is the real schema users with OpenCode activity will hit.

**Provider endpoint.** CodexBar calls OpenCode's SolidStart server functions through `GET/POST https://opencode.ai/_server` with `X-Server-Id`, `X-Server-Instance`, cookie, origin, and referer. The subscription function returns current `rollingUsage` and `weeklyUsage` (percent and reset seconds); the billing function returns current monthly usage/limit/balance. `GET https://opencode.ai/zen/go/v1/usage` is likewise current rolling/weekly state. No past buckets are exposed by these paths.

**Live source.** Existing OpenCode polls append current percentages when logged in.

## T3 Code's own Usage implementation

T3 Code makes no provider-history HTTP calls. The web view invokes its internal WebSocket RPC `server.getUsageSummary`, and each environment scans `~/.claude/projects/**/*.jsonl` and `~/.codex/sessions/**/*.jsonl`. It returns daily or hourly `(provider, model)` buckets containing uncached input, cached input, cache creation, output, reasoning, cost, record count, and session count. Its only network call is to LiteLLM's public model-pricing JSON for optional cost estimates; that is not a usage source. Burnrate adopts T3's parsers and provenance rules directly rather than calling T3.

## Implementation routing

| Provider | First-version backfill | Optional account truth | Existing-poll append |
| --- | --- | --- | --- |
| Claude | JSONL tokens | Admin API only if a future explicit Admin-key feature exists | quota `%` |
| Codex | JSONL token deltas | none | quota `%` |
| Cursor | none locally | filtered usage-events when manual cookie exists | quota `%` |
| Grok | completed-turn JSONL; signals only as session snapshot fallback | none | quota/credits `%` |
| OpenCode | SQLite messages, then legacy message JSON | none | quota `%` |

Backfill runs asynchronously, is idempotent by source event key, prunes rows older than 180 days, and never blocks the Now view. Percent and token series are queried and rendered on separately labeled axes.
