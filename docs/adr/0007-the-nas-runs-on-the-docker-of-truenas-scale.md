# The NAS runs on the Docker of TrueNAS SCALE

The NAS ran on Docker Desktop on the Admin's PC. It moves to TrueNAS SCALE, first in a Hyper-V virtual machine on that PC, then on a UGREEN NAS that will run TrueNAS SCALE instead of UGOS. The virtual machine rehearses the UGREEN: same operating system, same layout, same deploy.

## Decision

- **The Docker of TrueNAS**: the Host is TrueNAS SCALE itself. The deployer starts the NAS through the Docker socket of TrueNAS, as it did under Docker Desktop, and TrueNAS lists the services as apps it does not manage.
- **One pool, three datasets**: `data` holds the torrents tree and the media tree as plain folders, `appdata` holds the folder of each service, `jellynas` holds the clone of the repo and `.env`. The torrents and media trees are never child datasets, since a child dataset is another file system and would break the hardlinks of cross-seed.
- **The apps user**: every service runs as the `apps` user of TrueNAS, 568:568, which owns the three datasets.
- **No new variable**: the services already read every path, port, id and secret from `.env`. Only `DATA_ROOT`, `APPDATA_ROOT`, `PUID` and `PGID` change on the Host. The Tdarr workers stay seeded at 1 and are raised in its web UI.
- **The virtual machine**: 3 vCPU, 8 GB of static memory, a 32 GB boot disk and a 200 GB pool disk as VHDX files on the NVMe drive of the PC, and an external virtual switch, so the Host has its own address on the local network like the UGREEN will.

## Considered Options

- **A Linux virtual machine inside TrueNAS**: rejected. It needs nested virtualisation under Hyper-V and a second layer on the UGREEN, reaches the data over NFS, and SQLite databases corrupt over NFS, so the appdata would need a disk of its own.
- **Custom Apps of TrueNAS, one per service**: rejected. Their YAML lives in the TrueNAS database, outside git, so the repo would stop being the source of truth of the NAS and the deployer would have nothing to deploy.
- **UGOS Pro on the UGREEN**: rejected. It has no ZFS, so the pool of the virtual machine could not replicate to the UGREEN, and the rehearsal would test another system than the one that runs the NAS.
- **The Seagate USB drive passed to the virtual machine**: rejected. The data fits on the NVMe drive, and the USB drive stays free for a backup.

## Consequences

TrueNAS does not support containers it did not create, so a TrueNAS update can change its Docker under the NAS. The pool of the virtual machine sits on a single drive with no redundancy until the UGREEN. The move to the UGREEN is a ZFS replication of the pool, and the services see the same paths. Docker Desktop leaves the NAS, so the repo drops its Windows support once the virtual machine runs the NAS, and ADR 0006 follows.
