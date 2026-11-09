# Secrets are encrypted in git with SOPS

Supersedes the point of [ADR 0010](0010-doco-cd-deploys-main.md) that kept the secrets in `.env` on the Host, and its rejection of SOPS.

With the Declared settings of [ADR 0013](0013-the-settings-of-the-services-live-in-git.md), the repo holds everything needed to rebuild the NAS except its secrets. `.env` sits in clear on the Host, the archives of the appdata hold every API key in clear, and losing the Host loses `.env`. ADR 0010 rejected SOPS because a lost key would lock the NAS out of its own settings. That risk goes away when the file is encrypted for several keys.

## Decision

- **One encrypted file**: `secrets.env` at the root of the repo, encrypted with SOPS and age, replaces `/jellynas/.env` in `env_files` of `.doco-cd.yml`. `.env.example` stays in clear as the template.
- **Three recipients**: the key of the Host, read by doco-cd through `SOPS_AGE_KEY_FILE`, a key of the Admin kept in their password manager, and a key printed and kept offline. Any one of them decrypts the file, so losing one locks nothing.
- **doco-cd decrypts**: it decrypts every SOPS file of the revision it deploys, before the deploy.
- **No other encrypted file**: Configarr and the settings service read their secrets from environment variables interpolated from `secrets.env`.
- **The Clone keeps the key of the Host**, so the Admin can run `check` and `dump` by hand.
- **Encrypted archives**: backup encrypts each archive of the appdata with age for the same recipients, since the appdata holds every API key in clear.
- **A leaked key**: it is removed from the recipients with `sops updatekeys`, and every secret it could read is rotated.

## Considered Options

- **`.env` in clear on the Host**: rejected. It is the one piece of the NAS that is neither in git nor recoverable.
- **An external vault through doco-cd** (Vaultwarden, Bitwarden, 1Password, Infisical): rejected. The NAS would depend on that vault to start, and doco-cd only injects its values through interpolation.
- **Docker secrets mounted in `/run/secrets`**: rejected for now. They guard against a reader of `docker inspect` on the Host, who can read the appdata anyway.
- **A single key**: rejected. It is the lockout ADR 0010 feared.

## Consequences

Changing a secret means `sops secrets.env` on a machine that holds a key, then a commit. A key printed on paper is only useful if the Admin can find it. doco-cd must support SOPS in the release the NAS pins, which is checked before the switch.
