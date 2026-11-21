# Liens

## 🎬 Jellyfin

Films, séries, animés, livres et mangas.

**<https://jellyfin.${DOMAIN}:${CADDY_HTTPS_PORT}>**
-# Le :${CADDY_HTTPS_PORT} fait partie de l'adresse, à saisir comme serveur dans
les applis.

- **Android** ·
  [Jellyfin](https://play.google.com/store/apps/details?id=org.jellyfin.mobile)
- **Android TV** · [Jellyfin for Android
  TV](https://play.google.com/store/apps/details?id=org.jellyfin.androidtv)
- **iPhone, iPad, Apple TV** ·
  [Swiftfin](https://apps.apple.com/app/id1604098728)
- **PC** · le navigateur, plus simple

## 🍿 Seerr

Demander un film, une série ou un animé. Une fois la demande validée, il arrive
seul dans Jellyfin.

**<https://seerr.${DOMAIN}:${CADDY_HTTPS_PORT}>**
-# Dans le navigateur, connexion avec Sign in with Jellyfin.

## 🎧 Aurral

Toute la musique, en FLAC. Découvrir des artistes, en suivre un ou ajouter un
album, et recevoir des playlists faites pour toi.

**<https://aurral.${DOMAIN}:${CADDY_HTTPS_PORT}>**
-# Dans le navigateur, connexion avec le bouton SSO, même compte que Jellyfin.

Pour écouter sur ton téléphone ou ton PC, une appli libre et gratuite :

- **Android** · [Tempus](https://github.com/eddyizm/tempus)
- **iPhone** · [Amperfy](https://github.com/BLeeEZ/amperfy)
- **PC** · [Feishin](https://github.com/jeffvli/feishin), ou le navigateur

-# Crée d'abord un mot de passe d'appli dans Aurral, Settings > Profile > Set
password, différent de celui de ton compte. Dans l'appli, choisis un serveur
Subsonic, mets l'adresse d'Aurral, ton identifiant et ce mot de passe.
-# Relie ListenBrainz dans Aurral pour des recommandations selon ce que tu
écoutes.

## 📚 Livres et mangas

Ils se lisent dans Jellyfin, sur l'appli ou dans le navigateur. Le lecteur
Jellybook est également disponible sur Github, si tu veux creuser.
-# Pour en demander un, écris-moi dans ton salon.

## 🎶 Bot musique

Toute la musique d'Aurral dans un salon vocal Discord.
-# Réservé au rôle MusicDJ, et dans les serveurs où il est présent. Rejoins un
salon vocal, puis :

- `/play` · lecture aléatoire de toute la musique
- `/play <recherche>` · choisir un artiste, un album, une playlist ou un titre
- `/pause` · pause ou reprise
- `/skip` · passer au titre suivant
- `/stop` · vider la file et quitter le salon
- `/queue` · voir le titre en cours et la file
- `/volume <0-100>` · régler le volume
