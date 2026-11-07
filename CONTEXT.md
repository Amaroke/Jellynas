# Jellynas

A personal media server built on Jellyfin, shared with friends over the internet.

## Language

### People

**Admin**:
The owner of the NAS, the only person with access to the Automation stack.
_Avoid_: root, manager

**Friend**:
A person outside the household who has their own Account and reaches the NAS from the internet.
_Avoid_: user, guest, relative

**Account**:
The single identity of a Friend or the Admin, valid both for watching and for making Requests.
_Avoid_: profile

**Request**:
A movie or show asked for by a Friend or the Admin, waiting to be added to the Library.
_Avoid_: demand, ticket

### Library

**Library**:
The movies, shows, anime, music, books and manga served by Jellyfin.
_Avoid_: media collection, catalog

**Media tree**:
The folders of the data root that hold the Library.
_Avoid_: media folder, library folder

**Torrents tree**:
The folders of the data root where every torrent downloads and seeds, never read by Jellyfin.
_Avoid_: downloads folder

**Automation stack**:
The services that find, download, rename, subtitle and normalise media to file them into the Library.
_Avoid_: arr, arr stack

### Ratio

**C411**:
The private tracker of the Admin, the only one the Farm and the Zen reserve work for.
_Avoid_: the tracker

**Hit and run**:
A torrent of a private tracker removed before its Seeding goal, which gets the account on that tracker banned.
_Avoid_: early removal

**Seeding goal**:
The ratio or seeding time a torrent reaches before it may stop seeding, set per private tracker.
_Avoid_: seed limit, share limit

**Farm**:
The torrents the NAS seeds only to earn upload on C411, kept while they upload.
_Avoid_: ratio, ratio folder, ratio torrents

**Zen**:
The currency C411 pays for long seeding, traded for upload in its shop.
_Avoid_: bonus points

**Rare**:
Said of a C411 torrent whose seeders are few enough to count for the Swarm Keeper class of C411.
_Avoid_: dead, unpopular

**Zen reserve**:
The Rare C411 torrents the NAS keeps seeding for the Zen they pay, whatever they upload.
_Avoid_: rare pool, bonus torrents

**Twin**:
The same release, byte for byte, on a public tracker, downloaded there so that its C411 copy costs no download on C411.
_Avoid_: mirror, duplicate

**Source**:
How a C411 torrent came to the NAS: a paid grab, a Twin, a freeleech grab, a cross-seed of the Library, a Library download from C411, or a Rare torrent hunted for the Zen reserve. It never changes, even when the torrent later moves to the Zen reserve.
_Avoid_: origin, kind, pool

**Ledger**:
What every C411 torrent of the NAS downloaded and uploaded, removed torrents included.
_Avoid_: history, stats

**Deficit**:
How much more the Ledger downloaded than it uploaded. Its opposite is the **Surplus**.
_Avoid_: debt, balance

### Hosting

**NAS**:
All the services and data hosted for this project, whatever machine runs them.
_Avoid_: server, box, stack

**Host**:
The machine, or virtual machine, whose Docker runs the NAS. A Host is never called the NAS, even when it runs TrueNAS.
_Avoid_: server, NAS box, machine

**Clone**:
The copy of the repo on the Host that holds `.env` and starts doco-cd, but never deploys the NAS.
_Avoid_: checkout, working copy

**Public name**:
A name under the domain of the Admin that Friends reach from the internet.
_Avoid_: public URL, external address

**Lan name**:
A name under `lan.` of the domain of the Admin that only the local network reaches.
_Avoid_: local URL, internal address
