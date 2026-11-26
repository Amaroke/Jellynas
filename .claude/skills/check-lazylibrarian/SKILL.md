---
name: check-lazylibrarian
description: Show the books and manga of LazyLibrarian on the NAS as tables, what is missing and why, and launch searches. Use when asked for the state, the library, a check or a search of LazyLibrarian, the books or the manga.
---

# Check LazyLibrarian

Follow [the automation check](../../../docs/agents/automation-check.md) with the
app `lazylibrarian`.

LazyLibrarian searches through its providers: the Torznab ones come from
Prowlarr, the direct ones (LibGen and others) are its own, and an empty
`ENABLED` means off. The `policy` service holds its settings in
`services/policy/src/lazylibrarian.rs`, so a setting policy enforces is fixed
there, not on the NAS. Every path in `paths` must sit on one of the `mounts`.
