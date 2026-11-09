# The settings of the services live in git

Supersedes the point of [ADR 0010](0010-doco-cd-deploys-main.md) that left the settings made in the web UIs in the appdata root, outside git.

The repo is the source of truth of the NAS, yet most of what makes the NAS work was clicked by hand in the web UIs of the services and only lives in their appdata: root folders, download clients, naming, libraries, languages, logins. The README describes those steps, but nothing tells whether the NAS still matches them, a reinstall means clicking them all again, and nobody can list what each service lets us set.

## Decision

- **Declared settings**: every setting the Admin chooses is written in a file of the repo, in the native format of the tool that applies it. A setting that is not declared is left alone and never reported.
- **Configarr for Radarr, Sonarr, Prowlarr and Lidarr**: see [ADR 0015](0015-configarr-replaces-recyclarr.md). Its file is `configarr/config.yml`. Lidarr is experimental in Configarr: if it breaks, Lidarr moves to the settings service.
- **The settings service for the rest**: a Python container of the repo, `settings`, with one module per service, that talks to the API of each one. It covers Jellyfin, Seerr, qBittorrent, Bazarr, slskd, LazyLibrarian, Suwayomi, Tdarr, autobrr and Beszel. Its file is `settings/settings.yaml`.
- **AutoKuma for Uptime Kuma**: one monitor per service, from labels in the compose files, as today.
- **The other services keep their files and `.env`**: Caddy, Gluetun, Homepage, cross-seed, ratio, farm-report, the Music bot, backup, Dozzle, socket-proxy, FlareSolverr. They are already in git.
- **Three commands**, one entry point for all the tools, run in order: first-run wizards, then Prowlarr and the arr tools, then the other services.
  - `check` lists every Drift between the Declared settings and the NAS, Configarr included through its `DRY_RUN`, and checks that every variable a service needs is filled.
  - `apply` removes every Drift.
  - `dump` writes the whole configuration of each service, as its API returns it, to `dump/`, one JSON file per service, secrets masked. It is how the Admin sees every setting a service offers. `dump/` is ignored by git.
- **First-run wizards are automated**: the Jellyfin `/Startup` endpoints, the Jellyfin sign-in of Seerr, the `*__AUTH__*` variables of the arr tools. Each service keeps its own admin login. Every API key a service accepts from outside is set in advance, the others are created through the API once the admin login exists.
- **bootstrap**: a one-shot service writes, before their first start, the PBKDF2 hash of the qBittorrent password into `qBittorrent.conf` and the API key of Bazarr into its `config.yaml`, only when those files do not exist yet. qBittorrent and Bazarr wait for it to complete.
- **Apply at deploy**: a one-shot service, `settings-apply`, mounts the files of the Declared settings. doco-cd recreates it whenever one of them changes, so a change merged into `main` reaches the services without the Admin. `SETTINGS_APPLY=false` turns it into a `check`.
- **Check every night**: the settings service runs `check` every night and reports to a Push monitor of Uptime Kuma, declared through AutoKuma: up without Drift, down with the summary of the Drift. The alert goes to the alerts channel of Discord, as every other monitor.
- **Docs**: the README stops listing values to type. For each service it describes its screens and what each important setting does, and points to the file that holds the value.

## Considered Options

- **Buildarr for the arr tools**: rejected. Unmaintained since 2024, it supports neither Radarr v6 nor Sonarr v5, and it dropped its dry run.
- **Terraform with the devopsarr providers**: rejected. Same coverage as Configarr, but a Terraform state to keep and HCL to learn.
- **Templates of the config files of each service**, copied before the first start: rejected. They break when a service changes its format, and cannot tell a Drift.
- **One master file generating the config of Configarr and AutoKuma**: rejected. It would track the schema of each tool at every release.
- **Apply by hand from the Clone**: rejected. A change merged into `main` would not be the change the NAS runs.
- **Reporting every setting that differs from its default**: rejected. Defaults change from one release to the next, so every update would raise false Drift. `dump` already shows everything.
- **uptime-kuma-api for the monitors**: rejected. It stops at Uptime Kuma 1.23, and its fork for v2 targets a beta.

## Consequences

The settings service has to follow the API of ten services, and a release that renames a field shows up as a Drift or a failed `apply`. A setting changed by hand in a web UI is reverted at the next deploy that touches the Declared settings, and reported every night until then. The first deploy runs with `SETTINGS_APPLY=false`: the Declared settings are taken from a `dump` of the running NAS, and `apply` is only turned on once `check` finds no Drift, so the switch changes nothing.
