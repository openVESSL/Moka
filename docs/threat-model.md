# Threat model

What Moka protects, from whom, and what it does not cover. Each mitigation
listed as "tested" has an automated test that runs in CI.

## Assets

- **Credentials for other services**: indexer API keys, download client
  passwords, webhook addresses (which can embed tokens), an admin's own TMDB
  key (the built-in one ships in every binary and is not a secret).
- **Accounts**: passwords and sessions of the people using the server.
- **Files**: the media library and the download folder Moka can read and write.
- **The host network**: Moka makes outgoing requests to addresses users type in.

## Who might attack

1. Someone on the same network, or on the internet if the server is exposed
   through a reverse proxy, without an account.
2. A signed-in member (non-admin) trying to reach admin functions or other
   members' data.
3. A malicious or compromised indexer returning crafted feeds, links or redirects.
4. Someone who obtains a copy of the database file (for example, from a backup).

## Mitigations

| Risk | Mitigation | Tested |
|---|---|---|
| Anonymous access to the API | Every route requires a session or API key, except health, sign-in, first-run setup and logout. | yes |
| Credentials leaking through the API | Secrets are write-only: the API reports whether one is set, never its value. Webhook addresses are shown as scheme and host only. | yes |
| Database copy reveals credentials | Secrets are encrypted at rest (XChaCha20-Poly1305) with a key kept in `secret.key` or `MOKA_SECRET_KEY`, not in the database. | yes |
| Password guessing | Passwords hashed with Argon2id. After five failures an account name is locked for a minute, doubling up to 30 minutes. | yes |
| Stolen session tokens in the database | Session and API tokens are stored as SHA-256 hashes. Changing a password ends all sessions of that account. | yes |
| Members reaching admin functions | Each endpoint checks a permission bit (admin, manage requests, request, auto-approve, manage library, play). | partly |
| Requests to internal cloud services (SSRF) | Outgoing requests refuse link-local, cloud-metadata, multicast and unspecified addresses after DNS resolution and on every redirect. | yes |
| Path traversal through file names | Image and streaming routes accept only validated names; files are served from fixed folders. | yes |
| Clickjacking, content sniffing, injected scripts | `Content-Security-Policy` limited to the server's origin, `X-Frame-Options: DENY`, `nosniff`. | yes |
| Data loss on upgrade | The database is copied next to itself before a schema migration. | yes |

## Not covered (known limits)

- **Home-network addresses are reachable on purpose.** A self-hosted server
  usually talks to a download client on the same machine or LAN, so only the
  dangerous ranges are refused, not private networks.
- **TLS** is expected to be handled by a reverse proxy. The session cookie is
  not marked `Secure` by default because most installs run on plain HTTP in
  the home network.
- **Login throttling is in memory.** A restart clears it, and someone who knows
  an account name can keep it locked for up to 30 minutes.
- **The secret key sits on the same machine as the database.** Encryption
  protects copies of the database, not a fully compromised host.
- No two-factor authentication or single sign-on yet.
- `X-Forwarded-For` is not read anywhere, so it cannot be spoofed to bypass
  limits; the downside is that logs show the proxy's address.
