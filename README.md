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

Both roots live outside the repo, at the paths set in `DATA_ROOT` and `APPDATA_ROOT`. The data root holds a single tree, so that one folder holds every download and the whole Library:

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
mkdir -p "$DATA_ROOT"/torrents "$DATA_ROOT"/media/{movies,shows,anime,music,books,manga} "$APPDATA_ROOT"/seerr "$APPDATA_ROOT"/recyclarr
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

## qBittorrent

qBittorrent downloads torrents into the torrents tree, the only folder it sees, mounted under `/data/torrents`. It shares the network of Gluetun, whose firewall only lets traffic out through the VPN. When the VPN is down or not configured yet, qBittorrent has no connectivity at all, so your real IP never leaks.

### VPN

Until you choose a provider, leave every VPN value of `.env` empty. Gluetun then starts with a tunnel that leads nowhere and shows as unhealthy, which is expected.

Once you have a provider:

1. Find your provider on the [Gluetun providers page](https://github.com/qdm12/gluetun-wiki/tree/main/setup/providers) and note the values it asks for.
2. In `.env`, set `VPN_SERVICE_PROVIDER` to the provider name and `VPN_TYPE` to `wireguard` or `openvpn`.
3. For WireGuard, download a WireGuard config file from the provider and copy its `PrivateKey` into `WIREGUARD_PRIVATE_KEY` and its `Address` into `WIREGUARD_ADDRESSES`. For OpenVPN, set `OPENVPN_USER` and `OPENVPN_PASSWORD` to the manual setup credentials of the provider.
4. Optionally set `SERVER_COUNTRIES` to the countries you want to exit from.
5. Recreate Gluetun and qBittorrent:

   ```sh
   docker compose up -d
   ```

The line `Public IP address is` in `docker compose logs gluetun` shows the VPN address, never your home address. Gluetun then turns healthy.

### AirVPN

1. Sign in on the [AirVPN Client Area](https://airvpn.org/client/) and open the [Config Generator](https://airvpn.org/generator/).
2. Pick Linux as the system, WireGuard as the protocol, and the country you want to exit from, then generate and download the config file.
3. In `.env`, set `VPN_SERVICE_PROVIDER` to `airvpn`, `VPN_TYPE` to `wireguard`, and either `SERVER_COUNTRIES` to a country such as `Netherlands` or `SERVER_REGIONS` to `Europe` to let Gluetun pick among every European server. The keys work with any server.
4. From the config file, copy `PrivateKey` into `WIREGUARD_PRIVATE_KEY`, `PresharedKey` into `WIREGUARD_PRESHARED_KEY`, and only the IPv4 part of `Address` into `WIREGUARD_ADDRESSES`, for example `10.128.0.2/32`. Then delete the file, it holds your keys.
5. On the [ports page](https://airvpn.org/ports/), reserve a port, leave its local port empty, and set `VPN_FORWARDED_PORT` to it.
6. Recreate Gluetun and qBittorrent with `docker compose up -d`.

In qBittorrent, Tools > Options > Connection, set "Port used for incoming connections" to `VPN_FORWARDED_PORT` and uncheck "Use UPnP / NAT-PMP port forwarding from my router". Back on the ports page, the test button of the port then shows it as open while a torrent runs.

### Account

1. Read the temporary password qBittorrent printed at its first start:

   ```sh
   docker compose logs qbittorrent
   ```

2. Open `http://<NAS address>:<QBITTORRENT_PORT>` from a browser on the local network and sign in with `admin` and that password.
3. In Tools > Options > WebUI > Authentication, set a new username and a strong password stored in your password manager. Leave both "Bypass authentication" options unchecked.

### Save paths

In Tools > Options > Downloads, set "Default Save Path" to `/data/torrents`, uncheck "Keep incomplete torrents in", and save. Any other path would land inside the container and be lost.

### Downloads and seeding

Each finished torrent keeps seeding from the torrents tree for a while, to give back to other peers. qBittorrent stops it once it reaches its seeding goal, then the tool that sent it removes it. In Tools > Options, set:

| Screen | Setting | Value |
| --- | --- | --- |
| Downloads | Default Torrent Management Mode | Automatic, so each category picks its own save path |
| Downloads | Append .!qB extension to incomplete files | Checked, so no tool imports a file that is still downloading |
| BitTorrent | Privacy | DHT and PeX checked, Local Peer Discovery unchecked since the container has no local peers |
| BitTorrent | Torrent Queueing | 3 active downloads, 10 active uploads, 15 active torrents |
| BitTorrent | Seeding Limits | Ratio 2 and total seeding time 20160 minutes (14 days), whichever comes first, then "Stop torrent" |
| Advanced | Network interface | `tun0`, so qBittorrent only ever uses the VPN, on top of the Gluetun kill switch |

Optionally, in Speed, set a global upload limit to about 60% of your internet upload speed, so seeding never slows Jellyfin streams to Friends.

## Radarr

Radarr fulfils movie Requests: it searches the indexers, sends the torrent to qBittorrent, then copies the finished movie into the Library. It sees only the movies folder, under `/data/media/movies`, and the torrents tree, under `/data/torrents`.

### Account

1. Open `http://<NAS address>:<RADARR_PORT>` from a browser on the local network.
2. Radarr asks for an authentication method at the first visit. Pick "Forms (Login Page)", set "Authentication Required" to "Enabled", never "Disabled for Local Addresses", and enter a username and a strong password, distinct from the other tools, stored in your password manager.

### Media management

In Settings > Media Management, click "Show Advanced" at the top, then:

1. Under Root Folders, add `/data/media/movies`.
2. Check "Rename Movies".
3. Under Importing, uncheck "Use Hardlinks instead of Copy". Each import is a copy, so the torrent keeps seeding from the torrents tree while Tdarr normalises the Library file.
4. Under File Management, set "Recycling Bin" to `/data/media/movies/.recycle` and "Recycling Bin Cleanup" to `30` days. A deleted movie stays there for 30 days and can be moved back. Jellyfin and Radarr skip hidden folders, so it never shows up as a movie.

Create the recycle bin folder first, from the repo folder in bash:

```bash
set -a && . <(tr -d '\r' < .env) && set +a
mkdir -p "$DATA_ROOT"/media/movies/.recycle
```

### Download client

1. In qBittorrent, right-click "Categories" in the left panel, pick "Add category", and enter `radarr` as the category and `/data/torrents/movies` as the save path.
2. In Radarr, Settings > Download Clients, click "+" and pick qBittorrent.
3. Enter `gluetun` as the host, since qBittorrent shares its network, `8080` as the port, and the qBittorrent Account.
4. Set "Category" to `radarr`.
5. Check "Remove Completed": once qBittorrent stops a torrent at its seeding goal, Radarr removes it and its files from the torrents tree, and the movie stays in the Library.
6. Click "Test", then "Save".

Copy the API key from Settings > General > Security, Prowlarr and Seerr need it below.

## Prowlarr

Prowlarr holds the indexers once and pushes them to Radarr, and later to the other tools. FlareSolverr opens indexers protected by Cloudflare for it. Neither sees any media file.

### Account

Open `http://<NAS address>:<PROWLARR_PORT>` and set the authentication the same way as Radarr, with its own username and password.

### FlareSolverr

In Settings > Indexers, click "+" and pick FlareSolverr. Enter `flaresolverr` as the tag and `http://flaresolverr:8191/` as the host, then save. FlareSolverr has no web UI and is reachable by the other containers only.

### Indexers

In Indexers, click "Add Indexer", pick a tracker and fill in what it asks for, an Account for a private one. For an indexer behind Cloudflare, add the `flaresolverr` tag so its requests go through FlareSolverr. Click "Test", then "Save".

### Radarr sync

1. In Prowlarr, not Radarr, open Settings > Apps, click "+" and pick Radarr.
2. Leave "Sync Level" on "Full Sync".
3. Set "Prowlarr Server" to `http://prowlarr:9696` and "Radarr Server" to `http://radarr:7878`.
4. Paste the Radarr API key, click "Test", then "Save".

Every indexer then shows up in Radarr, Settings > Indexers, and follows any change made in Prowlarr.

## Movie Requests

In Seerr, Settings > Services, click "Add Radarr Server":

1. Check "Default Server" and enter `Radarr` as the name.
2. Enter `radarr` as the hostname, `7878` as the port, leave SSL off, and paste the Radarr API key.
3. Click "Test", then pick a quality profile, `/data/media/movies` as the root folder, and "Released" as the minimum availability.
4. Leave "Enable Automatic Search" checked and save.

An approved movie Request is then searched by Radarr, downloaded by qBittorrent into `/data/torrents/movies`, and copied into the movies folder, where Jellyfin picks it up.

## Sonarr

Sonarr fulfils show Requests and fetches new episodes on its own: it searches the indexers, sends the torrent to qBittorrent, then copies each finished episode into the Library. It sees only the shows and anime folders, under `/data/media/shows` and `/data/media/anime`, and the torrents tree, under `/data/torrents`.

### Account

Open `http://<NAS address>:<SONARR_PORT>` and set the authentication the same way as Radarr, with its own username and password.

### Media management

In Settings > Media Management, click "Show Advanced" at the top, then:

1. Under Root Folders, add `/data/media/shows` and `/data/media/anime`.
2. Check "Rename Episodes".
3. Under Importing, uncheck "Use Hardlinks instead of Copy". Each import is a copy, so the torrent keeps seeding from the torrents tree while Tdarr normalises the Library file.
4. Under File Management, set "Recycling Bin" to `/data/media/shows/.recycle` and "Recycling Bin Cleanup" to `30` days. Deleted episodes of both shows and anime stay there for 30 days and can be moved back.

Create the recycle bin folder first, from the repo folder in bash:

```bash
set -a && . <(tr -d '\r' < .env) && set +a
mkdir -p "$DATA_ROOT"/media/shows/.recycle
```

### Download client

1. In qBittorrent, add a category `tv-sonarr` with `/data/torrents/shows` as the save path.
2. In Sonarr, Settings > Download Clients, click "+" and pick qBittorrent.
3. Enter `gluetun` as the host, `8080` as the port, and the qBittorrent Account.
4. Set "Category" to `tv-sonarr`.
5. Check "Remove Completed": once qBittorrent stops a torrent at its seeding goal, Sonarr removes it and its files from the torrents tree, and the episodes stay in the Library.
6. Click "Test", then "Save".

Copy the API key from Settings > General > Security, Prowlarr and Seerr need it below.

### Sonarr sync

In Prowlarr, Settings > Apps, click "+" and pick Sonarr. Set "Prowlarr Server" to `http://prowlarr:9696` and "Sonarr Server" to `http://sonarr:8989`, paste the Sonarr API key, click "Test", then "Save". Every indexer with TV categories then shows up in Sonarr, Settings > Indexers. An indexer without TV categories, such as Internet Archive, is never synced to Sonarr.

## Show Requests

In Seerr, Settings > Services, click "Add Sonarr Server":

1. Check "Default Server" and enter `Sonarr` as the name.
2. Enter `sonarr` as the hostname, `8989` as the port, leave SSL off, and paste the Sonarr API key.
3. Click "Test". The quality profiles only load once the test succeeds. Pick one and `/data/media/shows` as the root folder.
4. Set "Anime Series Type" to "Anime", pick the same quality profile for anime and `/data/media/anime` as the anime root folder.
5. Check "Season Folders" to match Sonarr, leave "Enable Automatic Search" checked, and save.

An approved show Request is then searched by Sonarr, downloaded by qBittorrent into `/data/torrents/shows`, and copied into the shows folder, or the anime folder for an anime, where Jellyfin picks it up. New episodes of a monitored show follow on their own.

## Recyclarr

Recyclarr creates one quality profile in Radarr and one in Sonarr, so that most downloads already match the Library format and Tdarr rarely has to re-encode. It sees no media file and only talks to the Radarr and Sonarr APIs. Its config is `recyclarr/recyclarr.yml`, and it syncs again every day at midnight, so a change to the TRaSH Guides or to the config reaches both tools on its own.

Both profiles follow the French MULTi.VO profiles of the [TRaSH Guides](https://trash-guides.info/Radarr/radarr-setup-quality-profiles-french-en/):

- **Quality**: WEB and Bluray 1080p, Bluray 720p as a fallback, nothing above 1080p.
- **Audio**: a release with both French and the original audio scores higher, a release without the original audio is refused.
- **Codec**: an H265 release scores 400 more than the same release in H264. The guide penalises H265 at 1080p, so that rule is skipped. The bonus outweighs the gap between release groups but not the French audio bonus of 500, since Tdarr can re-encode a file but cannot add a missing audio track.
- **Size**: the guide sets a minimum size per quality sized for H264, which would refuse most H265 Bluray encodes. The Bluray minimums are lowered to the WEB ones.

### API keys

In `.env`, set `RADARR_API_KEY` and `SONARR_API_KEY` to the keys copied from Settings > General > Security in each tool, then start Recyclarr:

```sh
docker compose up -d
```

Sync right away instead of waiting for midnight:

```sh
docker compose exec recyclarr recyclarr sync
```

Each tool ends with a `Completed` line and no error. Run the same command after changing `recyclarr/recyclarr.yml`.

### Multi Languages

A `MULTi` release holds French and the original audio, but Radarr and Sonarr only know it if each indexer says so. Without this, every `MULTi` release is read as French only and refused for lacking the original audio.

In Radarr, Settings > Indexers, edit each indexer, click "Show Advanced", set "Multi Languages" to "Original" and "French", and save. Do the same in Sonarr. Prowlarr keeps this setting when it syncs, but an indexer added later in Prowlarr needs it too.

### Assign the profiles

Recyclarr creates the profile but never assigns it. In Radarr, the new profile is `[French MULTi.VO] HD Bluray + WEB`. In Sonarr, it is `[French MULTi.VO] HD Bluray + WEB (1080p)`.

1. In Radarr, Movies, click "Select Movies", select all, pick the profile under "Quality Profile" at the bottom, and save. Do the same in Sonarr, Series, for every show and anime.
2. In Settings > Profiles of each tool, delete the profiles that came with it, so a new movie or show can only get the Recyclarr one.
3. In Seerr, Settings > Services, edit Radarr and pick the profile as the quality profile. Edit Sonarr and pick it as both the quality profile and the anime quality profile.

## Verify the NAS

Run the verification script from the repo folder, in Git Bash on Windows:

```sh
./scripts/verify.sh
```

It checks the NAS from the outside and prints one line per promise. To prove the kill switch, it cuts the VPN for a few seconds and then restarts Gluetun and qBittorrent. A failing line names the service that broke it, and the script exits with an error if any check fails.

The script ends with the public addresses to test yourself. Open each one on a phone on 4G with wifi off: it must load over HTTPS without a certificate warning.

## Contributing

Before committing to this repo, enable the secret hook with [pre-commit](https://pre-commit.com):

```sh
pre-commit install
```

gitleaks then scans every commit and refuses it if it finds a secret.
