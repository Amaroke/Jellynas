# Jellynas

A personal media server built on Jellyfin, shared with friends over the
internet.

## Language

### People

**Admin**:
The owner of the NAS, the only person with access to the Automation stack.
_Avoid_: root, manager

**Friend**:
A person outside the household who has their own Account and reaches the NAS
from the internet.
_Avoid_: user, guest, relative

**Account**:
The single identity of a Friend or the Admin, kept in lldap, valid for watching,
for making Requests and in Aurral.
_Avoid_: profile

**Request**:
A movie or show asked for by a Friend or the Admin, waiting to be added to the
Library.
_Avoid_: demand, ticket

### Library

**Library**:
The movies, shows, anime, music, books and manga served by Jellyfin.
_Avoid_: media collection, catalog

**Media tree**:
The folders of the data root that hold the Library.
_Avoid_: media folder, library folder

**Torrents tree**:
The folders of the data root where every torrent downloads and seeds, never read
by Jellyfin.
_Avoid_: downloads folder

**Automation stack**:
The services that find, download, rename, subtitle and normalise media to file
them into the Library.
_Avoid_: arr, arr stack

### Discord

**Music bot**:
The Discord bot of the NAS that plays the music of the Library in a voice
channel of the Discord server of the Admin.
_Avoid_: the bot, jukebox

**DJ**:
A member of the Discord server of the Admin who holds the MusicDJ role and may
command the Music bot. A DJ needs no Account.
_Avoid_: listener, MusicDJ

**Queue**:
The tracks the Music bot will play next, in order.
_Avoid_: playlist, list

**Shuffle**:
The endless random play of the whole music of the Library, which plays every
track once before any repeats.
_Avoid_: random, radio

### Ratio

**C411**:
The private tracker of the Admin, the only one the Farm works for.
_Avoid_: the tracker

**Hit and run**:
A torrent of a private tracker removed before its Seeding goal, which gets the
account on that tracker banned.
_Avoid_: early removal

**Seeding goal**:
The ratio or seeding time a torrent reaches before it may stop seeding, set per
private tracker.
_Avoid_: seed limit, share limit

**Farm**:
The C411 torrents the NAS grabs only for what they earn on C411, the Account
ratio first and the Quests next, cross-seeds included. It grabs nothing more
once its disk budget is full, and never lets a torrent go.
_Avoid_: ratio, ratio folder, ratio torrents, cross-seed torrents, Zen reserve

**Switch**:
An on or off setting of the Farm that the Admin flips in Homepage, one per kind
of grab, per Quest farm and per cleaning.
_Avoid_: toggle, flag, button

**Public torrent**:
A torrent of a public tracker. It seeds two weeks whatever its ratio, and leaves
sooner once its C411 copy seeds the same files.
_Avoid_: Nyaa torrent, free torrent

**Zen**:
The currency C411 pays for long seeding, traded for upload in its shop.
_Avoid_: bonus points

**Rare**:
Said of a C411 torrent whose size and seeders fall in a bucket where the class
of the Admin on C411 adds a bonus to its note.
_Avoid_: dead, unpopular

**Quest farm**:
The grabbing of small C411 torrents into the Farm, behind a Switch, until the
count of a Quest reaches its target: the Seed count, or the seeds of one
category of C411.
_Avoid_: Volume hunt, Zen hunt, rare hunt

**Seed count**:
How many C411 torrents the NAS seeds at once, whatever their size, which the
Quests of C411 reward.
_Avoid_: seeds, torrent count

**Quest**:
One level of a challenge of C411, rewarded once, when the Seed count, the
seeding time, the note or the categories of the seeded torrents reach its
target. Only the seed and specialization paths count for the Farm.
_Avoid_: achievement

**Account ratio**:
The ratio C411 shows for the account of the Admin, every torrent ever included,
the upload credit bought with Zen counted as upload.
_Avoid_: real ratio, tracker ratio

**Ratio floor**:
The Account ratio of 1.2 under which the NAS spends no download on C411, not
even for the Library, freeleech and Twins aside.
_Avoid_: margin, buffer

**Ratio tier**:
An Account ratio the board marks: 0.8 where C411 blocks downloads, 1, the
Ratio floor and 2.
_Avoid_: threshold, level

**Twin**:
The same release, byte for byte, on a public tracker, downloaded there so that
its C411 copy costs no download on C411.
_Avoid_: mirror, duplicate

**Source**:
How a C411 torrent came to the NAS: a fresh paid grab, a free grab, a Library
download from C411, or a Quest farm grab. A free grab costs no download on
C411: a Twin, a freeleech grab or a cross-seed of the Library. It never changes,
even when a Library torrent later moves to the Farm.
_Avoid_: origin, kind, pool

**Ledger**:
What every C411 torrent of the NAS downloaded and uploaded, removed torrents
included.
_Avoid_: history, stats

**Deficit**:
How much more the Ledger downloaded than it uploaded. Its opposite is the
**Surplus**.
_Avoid_: debt, balance

### Hosting

**NAS**:
All the services and data hosted for this project, whatever machine runs them.
_Avoid_: server, box, stack

**Host**:
The machine, or virtual machine, whose Docker runs the NAS. A Host is never
called the NAS, even when it runs TrueNAS.
_Avoid_: server, NAS box, machine

**Clone**:
The copy of the repo on the Host that holds `.env` and starts doco-cd, but never
deploys the NAS.
_Avoid_: checkout, working copy

**Public name**:
A name under the domain of the Admin that Friends reach from the internet.
_Avoid_: public URL, external address

**Lan name**:
A name under `lan.` of the domain of the Admin that only the local network
reaches.
_Avoid_: local URL, internal address

**Network zone**:
A Docker network that groups the services one exposed service may reach:
Jellyfin with Caddy and lldap, Seerr with Caddy, Radarr and Sonarr, Aurral with
Caddy and Lidarr, Authelia with Caddy and lldap, every other service in the
internal zone. A zone only opens towards the exposed services, through their
Public names.
_Avoid_: VLAN, segment, subnet

**Declared settings**:
The settings of the services that the Admin chose and wrote in the repo, each in
the file of the tool that applies it. A setting that is not declared is left as
the service has it.
_Avoid_: config as code, desired state

**Drift**:
A Declared setting that the NAS no longer has, found by `check` and removed by
`apply`.
_Avoid_: diff, mismatch, deviation
