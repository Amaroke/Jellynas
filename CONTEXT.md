# NAS

Serveur multimédia personnel bâti sur Jellyfin, partagé avec des amis via internet, d'abord hébergé sur le PC puis destiné à une machine dédiée.

## Language

**NAS**:
L'ensemble des services et des données hébergés pour ce projet, quelle que soit la machine qui les fait tourner.
_Avoid_: serveur, box, stack

**Médiathèque**:
Les films, séries, musiques et livres servis par Jellyfin.
_Avoid_: bibliothèque, librairie

**Pile d'automatisation**:
Les services qui trouvent, téléchargent, renomment et sous-titrent les médias pour les ranger dans la Médiathèque.
_Avoid_: arr, stack arr

**Ami**:
Une personne extérieure au foyer qui possède son propre compte et accède au NAS depuis internet.
_Avoid_: utilisateur, proche, invité

**Admin**:
Le propriétaire du NAS, seul à accéder à la Pile d'automatisation.
_Avoid_: root, gestionnaire

**Compte**:
L'identifiant unique d'un Ami ou de l'Admin, valable à la fois pour regarder et pour faire des Demandes.
_Avoid_: login, profil

**Demande**:
Un film ou une série réclamé par un Ami ou par toi, en attente d'être ajouté à la Médiathèque.
_Avoid_: requête, request
