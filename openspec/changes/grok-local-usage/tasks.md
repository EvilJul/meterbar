## 1. Models & settings plumbing

- [x] 1.1 Add `grok` to `PROVIDER_IDS`, default order, `ProviderVisibility`, parse/normalize paths
- [x] 1.2 Settings load/save: visibility + order migration from 3-provider lists
- [x] 1.3 Unit tests for order normalize and visibility defaults including `grok`

## 2. Auth discovery

- [x] 2.1 Locate Grok Build / `grok login` credential paths (macOS + Linux candidates; env override for tests)
- [x] 2.2 Parse bearer + user id (or required fields) without logging secrets
- [x] 2.3 `is_configured` / has-auth helper for visibility `auto`
- [x] 2.4 Unit tests with temp fixture files

## 3. Billing provider

- [x] 3.1 Implement `providers/grok.rs`: GET `{proxy}/billing?format=credits` with auth headers
- [x] 3.2 Map new + legacy JSON → `UsageSnapshot` (percent, period end, error statuses)
- [x] 3.3 Fixture tests from grok-build-shaped responses (credits + legacy)
- [x] 3.4 Wire `refresh_grok` + `refresh_all` / AppState

## 4. Frontend

- [x] 4.1 Grok card in main board (progress + period + failure states)
- [x] 4.2 Settings: visibility control + order drag for four providers
- [x] 4.3 Optional open `https://grok.com/?_s=usage` (manage / needs_auth help)
- [x] 4.4 Provider refresh round includes Grok best-effort

## 5. Docs & validation

- [x] 5.1 README: Grok provider, auth via `grok login`, private proxy caveat, not xAI API billing
- [x] 5.2 `openspec validate grok-local-usage`
- [x] 5.3 Manual smoke: no auth → needs_auth; with `grok login` → percent card; other providers still refresh
