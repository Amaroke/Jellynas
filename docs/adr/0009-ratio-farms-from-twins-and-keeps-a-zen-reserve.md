# ratio farms from Twins and keeps a Zen reserve

ratio-grab and ratio-limit ran in two containers that shared a size limit and a category without seeing each other, and every grab cost ratio on C411, the only private tracker of the Admin, which sat at the edge of its ratio. C411 also pays Zen, 1 GB of upload each, for long seeding, with a class bonus for rare torrents. One script, ratio, now runs both pools.

## Decision

- **One script, one container**: `ratio/ratio.py` replaces ratio-grab and ratio-limit, with the standard library of Python only. Every 10 minutes it counts, drops Twins, cleans the Farm, cleans the Zen reserve, then grabs.
- **Twins**: for each fresh C411 episode with a leecher, ratio searches Nyaa for its episode and language, keeps a result of the same release group within 5 % of its size, and compares the length of every file in both torrent files. With a Twin, it downloads it from Nyaa, outside any daily quota or deficit pause. cross-seed searches C411 by the Nyaa title and misses Twins listed under a French title, so after an hour ratio adds the C411 torrent itself on the files of the Nyaa copy. qBittorrent tells cross-seed about each finished torrent through "Run external program on torrent finished", which ratio sets, so cross-seed adds the C411 copy while the swarm is still hungry. ratio then removes the Nyaa copy, the hardlink of cross-seed keeps the data.
- **Nyaa first in Radarr and Sonarr**: Prowlarr gives Nyaa priority 10 and C411 priority 25, so an anime Request downloads from Nyaa at equal quality and score, and cross-seed seeds it on C411.
- **The Farm**: the `ratio` and `cross-seed-link` categories, up to `RATIO_MAX_SIZE_GB`. A torrent that uploaded less than 0.2 times its size over the last 7 days leaves it, and so do the slowest uploaders when it is full.
- **The Zen reserve**: the `zen` category, up to `RATIO_ZEN_MAX_SIZE_GB`, for the Swarm Keeper class. It keeps rare C411 torrents, at most 24 seeders under 1 GB, 50 up to 100 GB, 199 beyond, whatever they upload, and comes first until it holds 15 torrents, the best bonus of the class. It fills from the Farm, a C411 torrent that leaves the Farm while still rare moves there, from Twins, and from C411 itself only once the surplus reaches `RATIO_ZEN_HUNT_MIN_SURPLUS_GB`.
- **Library torrents join the Zen reserve**: a rare C411 torrent of Radarr or Sonarr, 6 hours after it finished, moves to the `zen` category, out of reach of "Remove Completed". It is already paid, so its Zen cost no new download.
- **Freeleech is free**: a C411 release with a `downloadvolumefactor` of 0 is grabbed without quota nor deficit pause, and its download never counts in the ledger.
- **C411 peers**: C411 counts each seeder twice in the peers it gives Prowlarr, so ratio reads its leechers as the peers minus twice the seeders. Read as peers minus seeders, every swarm of C411 looked as hungry as it was seeded, and the Farm never saw a truly hungry one.
- **One ledger for the tracker**: the deficit counts every torrent whose announce URL holds `RATIO_TRACKER`, Radarr and Sonarr ones included, and keeps the torrents removed since, so it follows the real ratio of the Account.
- **ratio alone removes**: qbit_manage only tags torrents by tracker. ratio removes in the Farm and the Zen reserve, never before `RATIO_MIN_SEEDING_DAYS`.

## Considered Options

- **Removing autobrr**: rejected. It grabs nothing on C411, which announces on no IRC channel, but another tracker may join.
- **A week of observation before downloading Twins**: rejected. The Admin wanted the farm at once, and a Twin costs nothing on C411.
- **The Tanker class**: rejected. Its best bonus needs 750 torrents seeding at once, too much disk and load for the NAS. Swarm Keeper reaches its best at 15.
- **ratio adding the C411 copy itself**: rejected. cross-seed already checks that the files match and links them.
- **qbit_manage removing after 10 days without upload**: rejected. Two removers in the same categories would race, and its rule would throw away the rare torrents the Zen reserve keeps.
- **Hunting rare torrents on C411 whatever the ratio**: rejected while the Account sits at the edge of its ratio.

## Consequences

A Twin needs the same files on both trackers. French groups such as KAF and Tsundere-Raws post their episodes on both, under different titles: The Apothecary Diaries S03E06 VOSTFR of KAF holds the same 291 419 666 bytes on C411 and on Nyaa. A first search by the French title of C411 and the exact size found none, since Nyaa titles are English and Nyaa rounds its sizes to a tenth of a GiB, and Prowlarr hands Nyaa torrents as magnet links, so ratio fetches the torrent file from nyaa.si. On the newest releases of C411, 2 of 7 fresh episodes with a leecher had a Twin. Movies and shows other than anime have none. An adopted Library torrent takes its space twice. The ledger now counts the Requests that Radarr and Sonarr download from C411, so a large Request can pause paid grabs. A Zen torrent may upload nothing for months, its value is the Zen. The cross-seed hook depends on Gluetun letting qBittorrent reach cross-seed on the Compose network. Twins depend on C411 accepting cross-seeds from a public tracker. To tell which pool earns its disk, ratio keeps the Source of each C411 torrent in the ledger and logs once a day, per Source, the GB downloaded and uploaded, overall and over the last 7 days, and the days seeded in the Zen reserve. The same figures sit under `report` in its state file.
