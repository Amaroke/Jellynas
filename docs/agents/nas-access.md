# NAS access

How an agent reaches the Automation stack of the NAS to check it or fix it.
Everything goes through `scripts/check.sh` on the Clone, so the API keys stay in
the `.env` of the NAS and never reach the agent.

## Setup on the machine of the Admin

The skills call the NAS by the SSH alias `jellynas`. If `ssh jellynas true`
fails, the alias is missing from `~/.ssh/config`:

```text
Host jellynas
  HostName <LAN address of the NAS>
  User <admin user of the NAS>
```

The user needs passwordless `sudo` on the NAS. To run without a prompt each
time, `.claude/settings.local.json` allows `Bash(ssh jellynas *)`. That rule is
for the machine of the Admin only, never in the shared `.claude/settings.json`.

## Running a check

Pull the Clone, then run the check of one app. It prints one JSON report:

```sh
ssh jellynas 'sudo sh -c "cd /mnt/tank/jellynas \
  && git -c safe.directory=/mnt/tank/jellynas pull --ff-only -q \
  && sh scripts/check.sh radarr"'
```

The app is `radarr`, `sonarr`, `lazylibrarian` or `aurral`. The script refuses
to run when the Clone is not on `main` or has local changes. Report that to the
Admin as it is, since the Clone is theirs to clean.

The script builds `services/check` (the first build takes a few minutes), feeds
it the state, mounts and last 24 hours of logs of the app, Prowlarr, Gluetun,
qBittorrent and ratio, and runs it in the internal zone with the keys of `.env`.
For `aurral` it feeds the jobs of its database and the logs of slskd and
Gluetun instead, and reads the Library read-only. `check.sh repair-aurral`
rewrites the broken files of the Library, with no network.

## Calling an API

`check.sh api` sends one request with the key of the app and prints the reply,
with any key masked:

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api prowlarr GET /api/v1/indexer"
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api lazylibrarian GET 'cmd=readCFG&name=DOWNLOAD_DIR&group=General'"
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api radarr PUT /api/v3/indexer/1 '{\"id\":1}'"
```

A long body, like a whole indexer, does not fit in the arguments of `sudo` on
the NAS. Pass `-` instead and send the body on the standard input:

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api prowlarr PUT /api/v1/indexer/1 -" < indexer.json
```

The apps are `radarr`, `sonarr`, `prowlarr`, `lazylibrarian`, `aurral` and
`discord-posts`. Radarr and Sonarr answer under `/api/v3`, Prowlarr under
`/api/v1`, Aurral under `/api`. For LazyLibrarian the path is the query after
the key, starting with `cmd=`. `discord-posts` needs no key and answers under
`/api`, as the `publish-discord` skill shows. A `GET` reads, anything else
changes the NAS: send it only after the Admin said yes to that exact change.

## Reading further

For a fact the report lacks, read it in place, never a secret:

- `sudo docker logs -t --since 24h <container>` for logs.
- `sudo sqlite3 -readonly "file:/mnt/tank/appdata/<app>/<app>.db?immutable=1" "<query>"`
  for the databases of Radarr, Sonarr and Prowlarr, and
  `/mnt/tank/appdata/lazylibrarian/lazylibrarian.db` for LazyLibrarian.
- `services/<app>/compose.yaml` and the README of this repo for how each
  service is meant to be set up.
