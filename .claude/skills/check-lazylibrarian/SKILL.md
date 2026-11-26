---
name: check-lazylibrarian
description: Check LazyLibrarian on the NAS and trace each problem upstream to Prowlarr, ratio, Gluetun or qBittorrent. Use when asked for the state, health or a check of LazyLibrarian, the books or the manga.
---

# Check LazyLibrarian

Follow [the automation check](../../../docs/agents/automation-check.md) with the
app `lazylibrarian`.

LazyLibrarian searches through its providers: the Torznab ones come from
Prowlarr, the direct ones (LibGen and others) are its own, and an empty
`ENABLED` means off. The `policy` service holds its settings in
`services/policy/src/lazylibrarian.rs`, so a setting policy enforces is fixed
there, not on the NAS. Every path in `paths` must sit on one of the `mounts`.
