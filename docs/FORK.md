# Fork notes

This is a personal fork of [ntd4996/agentpet](https://github.com/ntd4996/agentpet),
Windows-first. Upstream is kept as a read-only reference so new upstream
features can be pulled in and then customised here.

## Remotes

| Remote | Repository | Use |
|---|---|---|
| `origin` | `trminhphuc15042002/agentpet` | push here (this fork is the source of truth) |
| `upstream` | `ntd4996/agentpet` | fetch upstream features only |

```bash
git fetch upstream
git merge upstream/main          # or: git rebase upstream/main <feature-branch>

git push                          # pushDefault = origin
```

## Branches

- `main` — the personal baseline (upstream + local fixes).
- `wip/*` — parked work that is not part of a buildable `main` yet.

## Releasing (Windows)

Push a tag; CI builds, signs, and attaches the installers plus `latest.json`:

```bash
# version lives in windows/src-tauri/tauri.conf.json
git tag win-v0.1.12 && git push origin win-v0.1.12
```

The updater endpoint is
`https://github.com/trminhphuc15042002/agentpet/releases/latest/download/latest.json`,
and the build is signed with the minisign key whose public half is in
`windows/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`).

The matching **private key is not in this repo**. It is stored locally at
`~/.tauri/agentpet.key` and in the GitHub Actions secret
`TAURI_SIGNING_PRIVATE_KEY`. **Back the private key up somewhere safe** — if it
is lost, future updates cannot be signed and auto-update breaks for everyone
already on a release.

## Branding status

Done:

- `identifier` → `io.github.trminhphuc15042002.agentpet`
- `publisher` / `homepage` → this fork
- updater `pubkey` + `endpoints` → this fork's releases
- `LICENSE` keeps the upstream MIT notice and adds this fork's copyright

Left as upstream (on purpose, or to revisit):

- `productName` / window title stay `AgentPet` so `%APPDATA%\AgentPet` (pets +
  care data) is preserved.
- Pet catalog + CSP still point at `pets.thenightwatcher.online`
  (upstream's public CDN). Self-host only if the catalog must be independent.
- The macOS app, landing site, and `web/` deploy are upstream-only and are not
  part of the Windows build.
