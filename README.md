# Jellynas

A personal media server built on Jellyfin, shared with Friends over the internet. Follow these steps in order, running every command from the repo folder.

## Prerequisites

- Docker: Docker Desktop on Windows, Docker Engine with the Compose plugin on Linux.
- Git, with Git Bash on Windows to run the verification script.

## Fill in the settings

```sh
cp .env.example .env
```

Open `.env` and fill in every value. Each variable is explained in `.env.example`. Wrap a value in double quotes if it contains a space. `.env` is ignored by git and must never be committed.

## Create the data and appdata roots

Both roots live outside the repo, at the paths set in `DATA_ROOT` and `APPDATA_ROOT`. The data root holds a single tree, so that moving a file from downloads to the Library is instant:

```text
<DATA_ROOT>
├── torrents
└── media
    ├── movies
    ├── shows
    ├── anime
    ├── music
    ├── books
    └── manga
```

Create it from the repo folder, in bash (Git Bash on Windows):

```bash
set -a && . <(tr -d '\r' < .env) && set +a
mkdir -p "$DATA_ROOT"/torrents "$DATA_ROOT"/media/{movies,shows,anime,music,books,manga} "$APPDATA_ROOT"/seerr
```

On Linux, make sure the user matching `PUID` and `PGID` owns both roots.

## Start the NAS

```sh
docker compose up -d
```

Each service is one line in the root `compose.yaml`. Comment a line to disable that service, then run the command again.

## Jellyfin

### First-run wizard

1. Open `http://<NAS address>:<JELLYFIN_PORT>` from a browser on the local network.
2. Pick the display language and create the Admin Account with a strong password stored in your password manager.
3. Skip the library step, it is done below.
4. Keep the metadata language and country you prefer.
5. Leave "Allow remote connections to this server" checked, Friends will reach Jellyfin from the internet later.

### Libraries

The media tree is mounted read-only under `/data/media`, so Jellyfin can never delete or change a file. In Dashboard > Libraries, add one library per folder:

| Library | Content type | Folder |
| --- | --- | --- |
| Movies | Movies | `/data/media/movies` |
| Shows | Shows | `/data/media/shows` |
| Anime | Shows | `/data/media/anime` |
| Music | Music | `/data/media/music` |
| Books | Books | `/data/media/books` |
| Manga | Books | `/data/media/manga` |

For each library, leave "Save artwork into media folders" and every "Metadata savers" option unchecked: Jellyfin cannot write into the Library and keeps its metadata in its appdata folder instead.

### Friend Accounts

Each Friend gets their own Account, so their watch history and progress are theirs. In Dashboard > Users, click "+" and:

1. Enter the Friend's name and a password, then give both to the Friend privately.
2. Grant access to all libraries.
3. Leave "Allow this user to manage the server" unchecked.
4. Check "Allow media downloading" so the Friend can download videos, books and manga to their phone.

## Public access

Friends reach Jellyfin from the internet over HTTPS. DuckDNS gives the NAS a free address that follows your home IP, and Caddy is the only service the router lets in. Caddy obtains the certificate on its own and refuses any hostname other than the ones set in `.env`, so the forwarded port can never lead to an admin tool. Caddy proves to Let's Encrypt that it owns the address through DuckDNS with your token, so the certificate needs no open port.

### DuckDNS

1. Sign in on [duckdns.org](https://www.duckdns.org) and add a subdomain. It must point to your home public IP, which DuckDNS fills in for you.
2. Copy the token shown at the top of the page.
3. In `.env`, set `DUCKDNS_SUBDOMAIN` to the subdomain without `.duckdns.org`, `DUCKDNS_TOKEN` to the token, and `JELLYFIN_HOSTNAME` to a name under your subdomain. `jellyfin.${DUCKDNS_SUBDOMAIN}.duckdns.org` reuses the subdomain set just above. DuckDNS resolves every name under your subdomain to the same IP.

The duckdns service then updates the address every five minutes. Your DuckDNS address and token stay in `.env` only.

### Router port forwards

Pick the HTTPS port Friends will use and set it in `CADDY_HTTPS_PORT`, or leave it empty for 443. A high port that no service uses by default, such as a random five-digit number, draws fewer automated scans. Stay below 49152: Windows reserves blocks of ports above it for Hyper-V and Docker, and `netsh interface ipv4 show excludedportrange protocol=tcp` lists them. Friends then add it to the address, for example `https://jellyfin.<subdomain>.duckdns.org:<port>`, which the Jellyfin apps accept.

On your router, forward that single TCP port to the local IP of the NAS, and nothing else. Set both the external and the internal port of the rule to `CADDY_HTTPS_PORT`, never the internal port to 443: Caddy listens on `CADDY_HTTPS_PORT` on the NAS. Every other port, including the Jellyfin port and the admin tools, stays reachable on the local network only. Give the NAS a fixed local IP in the router so the forward keeps working.

### Start Caddy and DuckDNS

```sh
docker compose up -d
```

The first start builds Caddy with its DuckDNS module, which takes a minute or two. After pulling a change to `caddy/Dockerfile`, add `--build` to rebuild it. Caddy then requests the certificate within a few minutes. Follow it with:

```sh
docker compose logs -f caddy
```

The line `certificate obtained successfully` means it worked.

### Jellyfin behind Caddy

In Dashboard > Networking, add `caddy` to "Known proxies" and save. Jellyfin then sees the real address of each Friend instead of the address of Caddy. Jellyfin looks up the address of Caddy when it starts, so restart it now and whenever Caddy is recreated:

```sh
docker compose restart jellyfin
```

## Seerr

Friends make Requests in Seerr, on its own public hostname, and log in with their Jellyfin Account. Seerr has no access to the Library files.

### Hostname

In `.env`, set `SEERR_HOSTNAME` to another name under your subdomain, for example `seerr.${DUCKDNS_SUBDOMAIN}.duckdns.org`. It uses the same HTTPS port, so the router needs no new rule. Recreate Caddy so it picks up the new name and obtains its certificate:

```sh
docker compose up -d
```

### Setup wizard

1. Open `http://<NAS address>:<SEERR_PORT>` from a browser on the local network.
2. Choose Jellyfin as the media server.
3. Enter `jellyfin` as the hostname, `8096` as the port, leave SSL off and the URL base empty.
4. Enter an email address and the username and password of the Jellyfin Admin Account. This Account becomes the Seerr Admin.
5. Sync the libraries and enable Movies, Shows and Anime.
6. Skip the Radarr and Sonarr step, they are added later.

### Settings

1. In Settings > General, set "Application URL" to `https://<SEERR_HOSTNAME>:<CADDY_HTTPS_PORT>`, without the port if it is 443.
2. In Settings > Network, check "Enable Proxy Support" so Seerr sees the real address of each Friend.
3. In Settings > Jellyfin, set "External URL" to `https://<JELLYFIN_HOSTNAME>:<CADDY_HTTPS_PORT>` so links to a movie open the public Jellyfin.
4. In Settings > Users, check "Enable Jellyfin Sign-In" and "Enable New Jellyfin Sign-In", uncheck "Enable Local Sign-In", and leave the default permissions on "Request".

A Friend then opens the Seerr address, picks "Sign in with Jellyfin" and enters their Jellyfin Account, with no second login to create.

## Verify the NAS

Run the verification script from the repo folder, in Git Bash on Windows:

```sh
./scripts/verify.sh
```

It checks the NAS from the outside and prints one line per promise. A failing line names the service that broke it, and the script exits with an error if any check fails.

The script ends with the public addresses to test yourself. Open each one on a phone on 4G with wifi off: it must load over HTTPS without a certificate warning.

## Contributing

Before committing to this repo, enable the secret hook with [pre-commit](https://pre-commit.com):

```sh
pre-commit install
```

gitleaks then scans every commit and refuses it if it finds a secret.
