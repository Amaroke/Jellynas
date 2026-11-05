# The NAS pulls main

The NAS was updated by hand: the Admin pulled the repo and ran `docker compose up -d`, and Dependabot proposed image updates without ever deploying them. The repo becomes the source of truth of the NAS, so that a change merged into `main` is the change the NAS runs.

## Decision

- **A deploy script on the host**: `scripts/deploy.sh` runs every 5 minutes from the Windows Task Scheduler or cron. It fast-forwards `main`, runs `docker compose up -d --build --remove-orphans`, and restarts the services of a folder when a file they mount changes. It keeps the last deployed commit in `.git/jellynas-deployed`, so a failed deploy is retried.
- **Pull, not push**: the NAS asks GitHub for changes, GitHub never reaches the NAS. Nothing opens on the router for the deploy.
- **No drift on the NAS**: the script refuses to deploy from another branch or while a tracked file differs from git. A change made on the NAS must go through a pull request.
- **Renovate instead of Dependabot**: Renovate reads the linuxserver tags with a versioning per image, merges minor and patch updates by itself once the CI passes, and waits for the Admin on major updates and on Gluetun and qBittorrent, which hold the VPN and its kill switch.
- **Secrets stay in `.env` on the NAS**: git never holds them, the Admin keeps `.env` up to date by hand when `.env.example` gains a variable.

## Considered Options

- **Komodo, Portainer or doco-cd**: rejected. They run Compose from a container, which resolves the bind mounts of the repo from paths inside that container, and those paths differ from the host under Docker Desktop on Windows. They also add a web UI and a database for what one script does.
- **A self-hosted GitHub Actions runner**: rejected. The repo is public, so a workflow from a pull request could run on the NAS.
- **Watchtower**: rejected. It updates images behind git, on tags the repo pins anyway, and knows nothing of the compose files or the scripts.
- **Dependabot**: rejected. It cannot compare most linuxserver tags, nor merge an update by itself.
- **Secrets encrypted in git with SOPS**: rejected for now. One Admin and one NAS do not need it, and a lost key would lock the NAS out of its own settings.

## Consequences

An update merged by Renovate reaches the NAS without the Admin, so a broken release can take a service down until it is reverted. The 3 days of `minimumReleaseAge` and the manual review of major updates limit that. A migration done by a service on its database is not undone by reverting its image. The settings made in the web UIs of the services stay outside git, the deploy never restores them.
