---
name: check-sonarr
description: Show the shows and anime of Sonarr on the NAS as tables, what is missing and why, and launch searches. Use when asked for the state, the library, a check or a search of Sonarr, the shows or the anime.
---

# Check Sonarr

Follow [the automation check](../../../docs/agents/automation-check.md) with the
app `sonarr`.

Sonarr files shows into `/data/media/shows`, anime into `/data/media/anime`, and
both into the Private libraries. The `missing` facts count the aired episodes
missing per series.
