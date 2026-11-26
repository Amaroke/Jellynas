# Automation check

The procedure the `check-radarr`, `check-sonarr` and `check-lazylibrarian`
skills follow. Reaching the NAS is in [nas-access.md](nas-access.md).

## 1. Run the check

Run the check of the app as [nas-access.md](nas-access.md) shows. Done when you
hold its JSON report, or the reason it could not run.

## 2. Trace every finding to its cause

The report holds `findings` (most severe first), `errors` (endpoints that did
not answer) and `facts`. Account for every finding and every error: each one
ends with a cause you can name and the evidence for it, or with "unknown" and
what you checked.

Trace upstream: a symptom in the app often starts in Prowlarr, ratio, Gluetun
or qBittorrent, whose state the report carries too. Read further with
`check.sh api` or the logs as [nas-access.md](nas-access.md) shows, and with
the README of this repo for how each service is meant to work.

Known causes, still reported every time with their cause:

- **C411 on the Paused sync profile.** ratio moved it there: the Account ratio
  is under the Ratio floor, or ratio cannot read the C411 profile. The `ratio`
  facts say which. C411 stays out of Radarr, Sonarr and LazyLibrarian until it
  is back.
- **Nyaa disabled until a time.** Prowlarr or the app backs off after failures:
  the indexer query limit set on Nyaa in Prowlarr, or 504 answers of nyaa.si.
  With C411 paused, that leaves no indexer at all.
- **qBittorrent answering 403.** It is reachable, it only refuses the request
  without login.
- **Sonarr without `indexerstatus`.** Sonarr has no such endpoint, its health
  checks carry the indexer failures instead.
- **Errors 521 or 522 from servarr.com.** The update and time servers of Radarr
  and Sonarr are down, nothing on the NAS.
- **Missing movies with the status announced.** Not released yet.
- **Failures "Manually marked as failed".** The Admin marked them.

## 3. Write the report

In the language of the conversation, with these sections:

1. **Verdict**: one line, working or not and why.
2. **Problems**: every finding and error, most severe first, each with its
   cause and evidence.
3. **Key figures**: library, missing, cutoff unmet, queue, failures over 7
   days, indexers, free space, warnings and errors of the logs.
4. **Proposed fixes**: for each problem with a fix, the exact change, as a
   `check.sh api` call or a command, and what it changes.

## 4. Apply a fix

Apply a fix only after the Admin said yes to that one fix. One fix at a time,
then read back the state it changed to show it took. A change that belongs in
the repo (a compose file, `policy`, `ratio`) is a pull request, not a call on
the NAS.
