# Requirements — ws-kit

Each requirement is tagged in code (`REQ-WSKIT-*` doc-comment markers) and
mapped to verifying tests. Security requirements are closed only when the
cited test exists and passes in CI. (REQ-WSKIT-001 — bearer parse
char-boundary safety — is cited historically in THREAT-MODEL.md.)

## REQ-WSKIT-200 — Origin validation on the WebSocket upgrade (CSWSH)

When `WsConfig::allowed_origins` is **non-empty**, a WebSocket upgrade
request whose `Origin` header is **missing** or **not in the allow-list**
must be rejected with **403 Forbidden before `on_upgrade`** — the socket is
never upgraded.

- Check entry: `origin_allowed_in_parts(&Parts, &allowed_origins)`
  (axum feature), decision fn `origin_allowed(Option<&str>, &[String])`.
- Comparison: **exact** match on scheme + host + port after normalization
  (`normalize_origin`: lowercase scheme/host, default ports `http`/`ws` 80
  and `https`/`wss` 443 omitted — `https://example.com:443` ==
  `https://example.com`).
- **No wildcard, suffix, prefix, or subdomain matching in v1** —
  `https://example.com` does not accept `https://evil-example.com`,
  `https://example.com.evil.com`, or `https://api.example.com`.
- Non-default ports are significant: `https://example.com:8443` != `https://example.com`.
- Bracketed IPv6 hosts normalize (`https://[2001:DB8::1]:443` ==
  `https://[2001:db8::1]`).

Tagged at: `src/origin.rs::origin_allowed`, `src/origin.rs::origin_allowed_in_parts`.

### Verifying tests

| Case | Test |
|------|------|
| allowed origin → 101 upgrade | `tests/integration.rs::origin_upgrade::req_wskit_200_allowed_origin_passes_upgrade` |
| disallowed origin → 403 pre-upgrade | `tests/integration.rs::origin_upgrade::req_wskit_200_disallowed_origin_rejected_403_pre_upgrade` |
| missing Origin (non-empty list) → 403 | `tests/integration.rs::origin_upgrade::req_wskit_200_missing_origin_rejected_403` |
| default-port normalization matches (`:443`) | `tests/integration.rs::origin_upgrade::req_wskit_200_port_normalized_match_passes`; unit `src/origin.rs::normalize_origin_default_ports_omitted` |
| no wildcard/suffix/subdomain match | `tests/integration.rs::origin_upgrade::req_wskit_200_no_wildcard_suffix_match`; unit `src/origin.rs::origin_allowed_no_wildcard_or_suffix_matching` |
| scheme + host case-insensitive; entries normalized identically | unit `src/origin.rs::origin_allowed_match_and_normalization` |
| scheme is significant (`http` != `https`) | unit `src/origin.rs::origin_allowed_scheme_matters` |
| Host header never stands in for Origin | unit `src/origin.rs::origin_allowed_in_parts_host_header_ignored` |
| malformed origin (no scheme) fails closed | unit `src/origin.rs::normalize_origin_no_scheme_fail_closed` |
| builder surface sets/replaces list | unit `src/config.rs::req_wskit_200_builder_sets_allowed_origins` |

## REQ-WSKIT-201 — Default empty allow-list = allow all (documented residual)

`WsConfig::default()` has `allowed_origins: []`, which **allows every
origin** — including upgrades with no Origin header — preserving pre-0.3.0
behavior so existing users do not break. This is a **documented, accepted
residual risk** (CSWSH remains possible until the integrator sets the list;
see THREAT-MODEL.md OPEN→residual note). The `WsConfig::allowed_origins`
field and builder docs carry a loud warning.

Tagged at: `src/config.rs::WsConfig.allowed_origins`.

### Verifying tests

| Case | Test |
|------|------|
| default config allows any origin and missing Origin → 101 | `tests/integration.rs::origin_upgrade::req_wskit_201_default_config_allows_all_origins` |
| default list is empty | unit `src/config.rs::req_wskit_201_default_allowed_origins_empty_allows_all`; decision fn `src/origin.rs::origin_allowed_empty_list_allows_all` |
