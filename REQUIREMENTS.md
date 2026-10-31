# Requirements: ws-kit
cited test exists and passes in CI. (REQ-WSKIT-001; bearer parse
char-boundary safety; is cited historically in THREAT-MODEL.md.)
## REQ-WSKIT-200: Origin validation on the WebSocket upgrade (CSWSH)
must be rejected with **403 Forbidden before `on_upgrade`**; the socket is
  and `https`/`wss` 443 omitted; `https://example.com:443` ==
- **No wildcard, suffix, prefix, or subdomain matching in v1** -
## REQ-WSKIT-201: Default empty allow-list = allow all (documented residual)
origin**; including upgrades with no Origin header; preserving pre-0.3.0
## REQ-WSKIT-300: Binary frames are opaque (never parsed as text/JSON)
## REQ-WSKIT-400: Compression is bounded and fails closed (0.4.0 `compression`)
   error (`WsError::Compression`); never guess, never lossy-coerce.
## REQ-WSKIT-500: Redis fan-out semantics are explicit (0.4.0 `redis`)
2. Apply remote messages **only to locally hosted rooms**; remote traffic
5. **Document delivery as at-most-once, best-effort**; no persistence, no