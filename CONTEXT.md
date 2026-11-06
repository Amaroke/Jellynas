# Jellynas

A personal media server built on Jellyfin, shared with friends over the internet, first hosted on the Admin's PC and later moved to a dedicated machine.

## Language

**NAS**:
All the services and data hosted for this project, whatever machine runs them.
_Avoid_: server, box, stack

**Host**:
The machine, or virtual machine, whose Docker runs the NAS. A Host is never called the NAS, even when it runs TrueNAS.
_Avoid_: server, NAS box, machine

**Library**:
The movies, shows, anime, music, books and manga served by Jellyfin.
_Avoid_: media collection, catalog

**Automation stack**:
The services that find, download, rename and subtitle media to file them into the Library.
_Avoid_: arr, arr stack

**Admin**:
The owner of the NAS, the only person with access to the Automation stack.
_Avoid_: root, manager

**Friend**:
A person outside the household who has their own Account and reaches the NAS from the internet.
_Avoid_: user, guest, relative

**Account**:
The single identity of a Friend or the Admin, valid both for watching and for making Requests.
_Avoid_: login, profile

**Request**:
A movie or show asked for by a Friend or the Admin, waiting to be added to the Library.
_Avoid_: demand, ticket
