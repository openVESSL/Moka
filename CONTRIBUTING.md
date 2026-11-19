# Contributing

Thanks for wanting to help. Koda is young, so issues describing real setups
(what you tried, what you expected, what happened) are as valuable as code.

## Before you start

- For anything larger than a small fix, open an issue first so we can agree
  on the approach.
- Security problems go through [SECURITY.md](SECURITY.md), not public issues.

## Development

Setup, commands and configuration are in the [README](README.md#development).
Before opening a pull request, these must pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir web build
```

If you change the HTTP API, run `pnpm --dir web gen:api` and commit the
regenerated files; CI fails when they are out of date.

## Guidelines

- Keep pull requests focused: one change, with tests for new behavior.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org)
  (`feat:`, `fix:`, `docs:` ...).
- New outgoing HTTP goes through `koda-net`; new routes require sign-in unless
  there is a reason they cannot (the route test will remind you).
- Never log secrets or full URLs that may contain API keys.

## Sign-off

Contributions are accepted under the project license (GPL-3.0). Please sign
off your commits (`git commit -s`) to certify the
[Developer Certificate of Origin](https://developercertificate.org/).
There is no contributor license agreement.
