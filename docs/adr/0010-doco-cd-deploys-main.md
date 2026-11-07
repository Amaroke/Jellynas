# doco-cd deploys main

Supersedes ADR 0006 (the NAS pulls main).

The repo is the source of truth of the NAS: a change merged into `main` is the change the NAS runs, and nothing else changes it. The NAS first ran a deployer of its own, a script in a container, because GitOps tools resolve the bind mounts of the repo from their own copy, and Docker Desktop, the first Host, named the Windows clone differently from Windows. Since [ADR 0007](0007-the-nas-runs-on-the-docker-of-truenas-scale.md), the Host is TrueNAS and its Docker runs on Linux, so that reason is gone.

## Decision

- **doco-cd**: a doco-cd container, in its own Compose project `jellynas-doco-cd`, polls `main` on GitHub every 3 minutes and deploys `compose.yaml` as the project `jellynas`, as set in `.doco-cd.yml`. It runs the first poll as soon as it starts.
- **Pull, not push**: the NAS asks GitHub for changes, doco-cd publishes no port, and nothing opens on the router.
- **One copy per revision**: doco-cd exports each revision of the repo into its data volume and deploys from there, so the bind mounts of the repo name a path the Docker of the Host sees. Every file the repo mounts into a service is read-only, so nothing the services write is lost from one revision to the next.
- **What a deploy recreates**: a service is recreated when its compose definition or image changes, or when a file of the repo it mounts changes, such as `ratio/ratio.py` or `caddy/Caddyfile`. A service with a `build:`, Caddy, is rebuilt at each deploy from the Docker build cache. `remove_orphans` removes a service commented out in `compose.yaml`.
- **Secrets stay in `.env` on the Host**: git never holds them. `.env` sits in the Clone on the Host, mounted read-only into doco-cd, which reads it to interpolate `compose.yaml`. The Admin updates it by hand when `.env.example` gains a variable.
- **The Clone**: the Host keeps a clone of `main`, which holds `.env` and starts doco-cd. It deploys nothing: the Admin runs the scripts of the repo and `docker compose logs`, `exec` and `restart` from it, never `docker compose up`.
- **doco-cd outside the NAS project**: doco-cd does not deploy itself. The Admin merges its updates and recreates it from the Clone.
- **Renovate merges by itself**: minor and patch updates merge once the CI passes and the release is 3 days old. Major updates, and those of Gluetun, qBittorrent and doco-cd, wait for the Admin.

## Considered Options

- **A deployer of our own**: rejected once the Host left Docker Desktop. doco-cd does what it did, plus the detection of changed mounts and build contexts per service.
- **Komodo or Portainer**: rejected. Both bring a web UI and a database to a NAS that needs neither.
- **The self-update of doco-cd**: rejected for now. It needs doco-cd inside the NAS project and a path to `.env` that no other variable gives, and a failed self-update is not retried until the next commit.
- **Webhooks from GitHub or a self-hosted Actions runner**: rejected. Webhooks need a port open on the router, and the repo is public, so a workflow from a pull request could run on the NAS.
- **Watchtower**: rejected. It updates images behind git, on tags the repo pins anyway.
- **Secrets encrypted in git with SOPS**: rejected for now. One Admin and one NAS do not need it, and a lost key would lock the NAS out of its own settings.

## Consequences

An update merged by Renovate reaches the NAS without the Admin, so a broken release can take a service down until it is reverted, and a migration a service made on its database is not undone by reverting its image. The settings made in the web UIs of the services stay in the appdata root, outside git. doco-cd reaches Docker through its socket and runs as root. A change of `.env` alone is no new revision: the Admin restarts doco-cd, whose first poll deploys again. The failed deploys show only in the logs of doco-cd until ntfy runs and doco-cd notifies it through Apprise.
