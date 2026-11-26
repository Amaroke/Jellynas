---
name: check-radarr
description: Show the movies of Radarr on the NAS as tables, what is missing and why, and launch searches. Use when asked for the state, the library, a check or a search of Radarr or of the movies.
---

# Check Radarr

Follow [the automation check](../../../docs/agents/automation-check.md) with the
app `radarr`.

Radarr files movies into `/data/media/movies` and the Private libraries. A movie
is missing only once released: weigh `status` and `isAvailable` in the
`missing` facts.
