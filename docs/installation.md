# Installation guide

Follow these steps in order to build the NAS from scratch. Every later piece of the NAS adds its own section here.

## Prerequisites

- Docker: Docker Desktop on Windows, Docker Engine with the Compose plugin on Linux.
- Git, with Git Bash on Windows to run the verification script.
- [pre-commit](https://pre-commit.com), to block any commit containing a secret.

## Clone the repo and enable the secret hook

```sh
git clone https://github.com/Amaroke/Jellynas.git
cd Jellynas
pre-commit install
```

From now on, gitleaks scans every commit and refuses it if it finds a secret.

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
    ├── music
    └── books
```

Create it from the repo folder, in bash (Git Bash on Windows):

```bash
set -a && . <(tr -d '\r' < .env) && set +a
mkdir -p "$DATA_ROOT"/torrents "$DATA_ROOT"/media/{movies,shows,music,books} "$APPDATA_ROOT"
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
| Music | Music | `/data/media/music` |
| Books | Books | `/data/media/books` |

For each library, leave "Save artwork into media folders" and every "Metadata savers" option unchecked: Jellyfin cannot write into the Library and keeps its metadata in its appdata folder instead.

### Friend Accounts

Each Friend gets their own Account, so their watch history and progress are theirs. In Dashboard > Users, click "+" and:

1. Enter the Friend's name and a password, then give both to the Friend privately.
2. Grant access to all libraries.
3. Leave "Allow this user to manage the server" unchecked.
4. Check "Allow media downloading" so the Friend can download movies and books to their phone.

## Verify the NAS

Run the verification script from the repo folder, in Git Bash on Windows:

```sh
./scripts/verify.sh
```

It checks the NAS from the outside and prints one line per promise. A failing line names the service that broke it, and the script exits with an error if any check fails.
