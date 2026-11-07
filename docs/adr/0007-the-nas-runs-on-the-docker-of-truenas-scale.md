# The NAS runs on the Docker of TrueNAS SCALE

The NAS runs on TrueNAS SCALE, on a machine or a virtual machine, so that its data sits on ZFS with snapshots and replication, and its services stay in git.

## Decision

- **The Docker of TrueNAS**: the Host is TrueNAS SCALE itself. doco-cd starts the NAS through the Docker socket of TrueNAS, and TrueNAS lists the services as apps it does not manage.
- **One pool, three datasets**: `data` holds the torrents tree and the media tree as plain folders, `appdata` holds the folder of each service, `jellynas` holds the Clone and `.env`. The torrents and media trees are never child datasets, since a child dataset is another file system and would break the hardlinks of cross-seed.
- **The apps user**: every service runs as the `apps` user of TrueNAS, 568:568, which owns the three datasets.

## Considered Options

- **A Linux virtual machine inside TrueNAS**: rejected. It adds a layer of virtualisation and reaches the data over NFS, where SQLite databases corrupt, so the appdata would need a disk of its own.
- **Custom Apps of TrueNAS, one per service**: rejected. Their YAML lives in the TrueNAS database, outside git, so the repo would stop being the source of truth of the NAS.
- **A NAS system without ZFS, such as UGOS Pro**: rejected. It has no snapshots nor replication to move the pool to another machine.

## Consequences

TrueNAS does not support containers it did not create, so a TrueNAS update can change its Docker under the NAS. Moving the NAS to another machine is a ZFS replication of the pool, and the services see the same paths. The repo supports no Windows Host.
