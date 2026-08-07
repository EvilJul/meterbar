## Context

See proposal.md — Why. Meterbar already has Cursor (local vscdb / Cookie), Codex (`codex app-server`), DeepSeek (API key). Grok Build OSS ([xai-org/grok-build](https://github.com/xai-org/grok-build)) shows consumer usage via:

- ACP extension `x.ai/billing` → `GET {cli_chat_proxy}/billing?format=credits`
- Default proxy: `https://cli-chat-proxy.grok.com/v1`
- Auth: Bearer from `grok login` + `x-userid` + `X-XAI-Token-Auth`
- Prefer `creditUsagePercent` + `currentPeriod`; legacy `used`/`monthlyLimit` (cents)

No stable public SuperGrok usage REST; web UI is Settings → Usage / `https://grok.com/?_s=usage`.

## Goals / Non-Goals

**Goals:**

- Fourth provider `grok` on the board with week-pool % + reset time
- Discover local grok auth first; best-effort refresh
- Visibility/order plumbing for four providers
- Fixture-tested mapping from billing JSON shapes (new + legacy)

**Non-Goals:**

- Full OAuth device flow UI in v1 (prefer reading existing `grok login` store)
- Product breakdown bars (`productUsage` unused by CLI)
- Merging xAI developer console pay-as-you-go with SuperGrok pool
- Guaranteeing private endpoint stability forever

## Decisions

### 1. Provider id and UX

- id: `grok`; display name: `Grok` or `SuperGrok` (prefer tier string if present, else `Grok`)
- Card: progress from `percentUsed`; subtitle period end; optional prepaid note
- Default order append: `…, grok`

### 2. Auth discovery (v1)

- Locate Grok Build / grok CLI credential files under platform-known paths (probe multiple candidates; env override e.g. `USAGES_GROK_AUTH_PATH` for tests)
- Extract bearer + user id (or whatever the proxy requires) **in memory only** for the request
- Optional later: dual-write Meterbar copy if paste UX needed
- `is_configured` ≡ discovery success (or stored optional paste)

### 3. HTTP client

- Reuse `reqwest` pattern from DeepSeek/Cursor
- Headers mirror grok-build `billing.rs` (Bearer, token-auth header, x-userid, client version if required)
- Timeout ~10–15s; map 401/403 → `needs_auth`

### 4. Snapshot mapping

```
percent_used = creditUsagePercent ?? used/limit
period_end = currentPeriod.end ?? billingPeriodEnd
status = ok | needs_auth | network_error | parse_error
```

- Do not invent fake 0% on missing config
- Unit may be `Unknown` when only percent exists

### 5. Commands & refresh

- `refresh_grok` command + include in `refresh_all` / frontend provider round
- Parallel-safe with existing Mutex AppState patterns

### 6. Settings schema

- `ProviderVisibility.grok`
- `PROVIDER_IDS` / normalize_order include `grok`
- Migration: 3-id lists append `grok`

### 7. Open manage URL

- Settings or card action → opener plugin → `https://grok.com/?_s=usage`

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| Private proxy changes | Isolate in `providers/grok.rs`; fixture tests; clear error states |
| Credential path drift | Multiple candidates + env override |
| Token in logs | Scrub; dual-write rules from credentials module |
| User has SuperGrok but never ran `grok login` | needs_auth + open Usage URL copy |

## Migration Plan

1. Specs + models visibility/order
2. Discover auth + billing fetch + map
3. Wire commands / panel UI
4. Docs

No migration of secrets; empty Grok until discovery works.

## Open Questions

- Exact on-disk layout of `grok login` credentials (confirm paths during implement by reading grok-build auth module).
- Whether v1 needs paste fallback if discovery is hard (default: discovery-only first).
