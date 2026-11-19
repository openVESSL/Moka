# Security policy

Koda stores credentials for your indexers and download client and has
access to your media folders, so security reports are taken seriously.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private reporting instead:
**Security → Report a vulnerability** on this repository.

Include what you found, how to reproduce it, and the version (`/api/v1/health`
shows it). You will get a reply within 7 days. Once a fix is released, an
advisory is published with credit to the reporter unless you prefer otherwise.

## Supported versions

Koda is in early development (0.x). Only the latest release receives fixes.

## Scope

In scope: the server, its HTTP API, the web app, the Docker image.
Out of scope: vulnerabilities in third-party services Koda connects to
(download clients, indexers, metadata providers), and attacks that require
an attacker who already has an admin account.

See [docs/threat-model.md](docs/threat-model.md) for what Koda protects and how.
