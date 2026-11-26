---
name: check-sonarr
description: Check Sonarr on the NAS and trace each problem upstream to Prowlarr, ratio, Gluetun or qBittorrent. Use when asked for the state, health or a check of Sonarr, the shows or the anime.
---

# Check Sonarr

Follow [the automation check](../../../docs/agents/automation-check.md) with the
app `sonarr`.

Sonarr files shows into `/data/media/shows`, anime into `/data/media/anime`, and
both into the Private libraries. The `missing` facts count the aired episodes
missing per series.
