# Configarr replaces Recyclarr

Supersedes the choice of Recyclarr as the tool of [ADR 0002](0002-recyclarr-prefers-french-h265-1080p.md). The profiles it decides stay the same.

[ADR 0013](0013-the-settings-of-the-services-live-in-git.md) declares the settings of Radarr, Sonarr, Prowlarr and Lidarr. Recyclarr only syncs quality profiles, custom formats and quality sizes. Configarr does that too, from the same TRaSH Guides and Recyclarr templates, and also covers media management, naming, root folders, download clients, delay profiles, notifications, host settings, and the indexers, applications and sync profiles of Prowlarr, with a dry run that prints the diff.

## Decision

- **Configarr alone**: it applies the profiles of ADR 0002 and every other Declared setting of Radarr, Sonarr, Prowlarr and Lidarr, from `configarr/config.yml`. Recyclarr and its config leave the repo.
- **The same profiles**: the migration keeps the profiles of ADR 0002 as they are. A `DRY_RUN` of Configarr against the NAS shows no change on the profiles before Recyclarr is removed.
- **Lidarr is experimental** in Configarr. It stays there as long as the nightly `check` finds no false Drift.

## Considered Options

- **Recyclarr for the profiles, Configarr for the rest**: rejected. Two tools writing the same profiles end up fighting.
- **Buildarr or Terraform**: see ADR 0013.

## Consequences

The profiles of ADR 0002 now follow the releases of Configarr, not those of Recyclarr. The nightly sync of Recyclarr is replaced by `settings-apply` at each deploy and the nightly `check`.
