# doco-cd deploys main

[ADR 0006](0006-the-nas-pulls-main.md) gave the NAS a deployer of its own, a container that ran `scripts/deploy.sh`, because GitOps tools resolve the bind mounts of the repo from their own copy, and Docker Desktop, the first Host, named the Windows clone differently from Windows. Since [ADR 0007](0007-the-nas-runs-on-the-docker-of-truenas-scale.md), the Host is TrueNAS and its Docker runs on Linux, so that reason is gone, and the deployer is code the repo has to test and maintain.

## Decision

- **doco-cd**: a doco-cd container, in its own Compose project `jellynas-doco-cd`, polls `main` on GitHub every 3 minutes and deploys `compose.yaml` as the project `jellynas`, as set in `.doco-cd.yml`. It runs the first poll as soon as it starts.
- **One copy per revision**: doco-cd exports each revision of the repo into its data volume and deploys from there, so the bind mounts of the repo name a path the Docker of the Host sees. Every file the repo mounts into a service is read-only, so nothing the services write is lost from one revision to the next.
- **What a deploy recreates**: a service is recreated when its compose definition or image changes, or when a file of the repo it mounts changes, such as `ratio/ratio_grab.py` or `caddy/Caddyfile`. A service with a `build:`, Caddy, is rebuilt at each deploy from the Docker build cache, and recreated when a file of its build context changes. `remove_orphans` removes a service commented out in `compose.yaml`.
- **`.env` stays on the Host**: `.env` stays in the clone of the Admin on the Host, outside git, and is mounted read-only into doco-cd, which reads it to interpolate `compose.yaml`.
- **The clone of the Admin**: the Host keeps a clone of `main`, which holds `.env` and starts doco-cd. It deploys nothing: the Admin runs the scripts of the repo and `docker compose logs`, `exec` and `restart` from it, never `docker compose up`.
- **doco-cd outside the NAS project**: doco-cd does not deploy itself. Renovate opens the update of its image, and the Admin merges it and recreates doco-cd from the clone.
- **Pull, not push**: as in ADR 0006, the NAS asks GitHub for changes, doco-cd publishes no port, and nothing opens on the router.

## Considered Options

- **Keep the deployer**: rejected. It was written only for Docker Desktop, and doco-cd covers what it did, plus the detection of changed mounts and build contexts per service instead of per folder.
- **Komodo or Portainer**: rejected. Both bring a web UI and a database to a NAS that needs neither. ADR 0006 said doco-cd brings them too, which was wrong: doco-cd has neither.
- **The self-update of doco-cd**: rejected for now. It needs doco-cd inside the NAS project and a path to `.env` that no other variable gives, and a failed self-update is not retried until the next commit. Recreating doco-cd by hand on its rare updates is cheaper.
- **Webhooks from GitHub**: rejected. They need a port open on the router, for a deploy that polling makes within 3 minutes.

## Consequences

doco-cd reaches Docker through its socket and runs as root, like the deployer. A change of `.env` alone is no new revision: the Admin restarts doco-cd, whose first poll deploys again. The deploy no longer refuses to run while the clone of the Admin differs from git, since it never reads the clone. The failed deploys show only in the logs of doco-cd until ntfy runs and doco-cd notifies it through Apprise.
