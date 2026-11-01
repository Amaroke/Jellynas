# Normalisation de toute la Médiathèque en MKV H265 1080p par Tdarr

Chaque vidéo téléchargée est réencodée par Tdarr en MKV H265 1080p, en gardant uniquement l'audio VF et VO et les sous-titres FR et EN intégrés au fichier. On veut un format unique, compact et lisible directement par la plupart des appareils, y compris hors ligne sur téléphone avec les sous-titres. Le réencodage est destructif : le fichier d'origine est remplacé et chaque passage perd de la qualité, donc revenir en arrière impose de retélécharger.

## Considered Options

- **Préférer seulement le bon format au téléchargement (Recyclarr)** : sans perte, mais les fichiers restent hétérogènes quand la source idéale n'existe pas. Gardé en complément pour limiter les réencodages.
- **MP4 plutôt que MKV** : meilleure compatibilité navigateur, mais ne gère pas tous les formats de sous-titres ni proprement plusieurs pistes audio.
- **H264 plutôt que H265** : lu partout sans transcodage, mais fichiers bien plus lourds.

## Consequences

La machine qui fait tourner Tdarr doit encoder le H265 matériellement : la RTX 5070 du PC aujourd'hui, un processeur Intel récent avec QuickSync sur la future machine dédiée.
