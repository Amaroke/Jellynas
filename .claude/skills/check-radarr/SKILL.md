---
name: check-radarr
description: Check Radarr on the NAS and trace each problem upstream to Prowlarr, ratio, Gluetun or qBittorrent. Use when asked for the state, health or a check of Radarr or of the movies.
---

# Check Radarr

Follow [the automation check](../../../docs/agents/automation-check.md) with the
app `radarr`.

Radarr files movies into `/data/media/movies` and the Private libraries. A movie
is missing only once released: weigh `status` and `isAvailable` in the
`missing` facts.
