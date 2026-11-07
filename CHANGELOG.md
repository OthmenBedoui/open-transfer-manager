# Historique

## Non publié — envoi de fichiers (phase A)

- Nouvelle section **Send** : glisser-déposer des fichiers ou dossiers (n'importe où dans la fenêtre),
  choisir plusieurs destinations, envoi en parallèle avec progression, arrêt, nouvel essai,
  liens de partage (S3 pré-signés 7 jours, Nextcloud…).
- Nouvelle section **Connectors** : SFTP (agent SSH, fichier de clé ou mot de passe), FTP/FTPS,
  WebDAV (Nextcloud, ownCloud) et stockage compatible S3 ; bouton « Test connection ».
- « Send to… » dans le menu d'un téléchargement terminé.
- Moteur d'envoi : rclone, installé par OTM depuis downloads.rclone.org avec vérification SHA-256
  (`otm update-rclone`). La configuration rclone personnelle de l'utilisateur n'est jamais lue.
- Mots de passe et clés dans le Gestionnaire d'identification Windows, le Trousseau macOS ou
  le Secret Service Linux (`secret-tool`) ; sinon un fichier privé (0600), indiqué dans l'interface.
- Ligne de commande : `otm connectors`, `otm send FICHIER... --to NOM[,NOM] [--link]`.

## 0.2.7 — ouvrir le fichier et son dossier

- « Show in folder » ouvre enfin le bon dossier sous Windows et sélectionne le fichier
  (avant : l'Explorateur ouvrait Documents dès que le chemin contenait des espaces).
- Double-clic / Open sur un fichier déplacé ou supprimé : message clair et ouverture de son dossier,
  au lieu de ne rien faire.
- Retirer un doublon avec « supprimer le fichier » ne supprime plus un fichier encore utilisé
  par une autre ligne de la liste.

## 0.2.6 — sites de confiance

- Case « Toujours autoriser les téléchargements depuis ce site » dans la confirmation des liens du navigateur.
- Les sites de confiance (et leurs sous-domaines) démarrent sans confirmation ; les autres demandent toujours.
- Site déterminé par la page d’origine (Referer), sinon par le lien ; domaines imitant un site de confiance refusés.
- Liste visible et modifiable dans Réglages → Browsers, vide par défaut.
- Une instance headless accepte aussi les liens des sites de confiance.
- Installateur en une commande pour Linux et macOS (`install.sh`) : vérification SHA-256,
  entrée de menu ou app macOS, ffmpeg proposé, yt-dlp, pont navigateur, désinstallation,
  compilation automatique depuis les sources sur les systèmes sans binaire (Intel Mac, ARM, anciennes distributions).
- README en anglais avec couverture et installation par système ; workflow de publication des binaires.

## 0.2.5 — pochette et paroles en option

- Nouveau réglage « Add cover art and lyrics to MP3 files » (Réglages → Vidéo & audio), désactivé par défaut.
- Désactivé : aucune requête vers LRCLIB, MusicBrainz ou Cover Art Archive.
- Activé : fonctionne pour toutes les qualités MP3 (High, Standard, Small), plus seulement 320 kb/s.
- L'API locale indique l'état réel du réglage (`capabilities.music_enrichment`).

## 0.2.4 — enrichissement automatique High Audio

- Recherche LRCLIB et pochette MusicBrainz / Cover Art Archive uniquement pour le MP3 High 320 kb/s individuel.
- Conservation du téléchargement si les données ou les services sont indisponibles ; résultat visible dans la liste.
- Suppression du format Gold distinct ; couleur dorée après intégration réussie des deux tags.

## 0.2.3 — option musicale dorée

- Format MP3 avec pochette et paroles, proposé uniquement lorsque les données de la source et l’encodeur sont disponibles.
- Affichage doré dans les formats, la liste native et celle de l’extension.
- Clic sur le format doré : démarrage direct sans fenêtre de détection ni confirmation supplémentaire.
- Validation de la disponibilité avant transfert, écriture ID3 vérifiée sur copie et conservation de l’audio original en cas d’échec.
- Maintien des confirmations ordinaires pour les autres types de téléchargement.

## 0.2.2 — noms arabes et confirmation des téléchargements

- Police Noto Sans Arabic embarquée dans le binaire, repli dans les champs de saisie et rendu bidirectionnel des noms affichés.
- Noms et chemins conservés en Unicode logique sur disque ; chiffres et suffixes latins restent lisibles.
- Confirmation native pour les liens du navigateur et les URL transmises au lancement de la fenêtre.
- Demandes multiples traitées successivement, annulation sans entrée ni fichier créé.
- Cookies/en-têtes et options média conservés jusqu’à confirmation ; identifiants retirés lors d’un changement d’URL.
- Messages de l’extension adaptés au nouvel état « en attente de confirmation ».
- Protection contre l’envoi à une ancienne application ou une instance headless incapable d’afficher la confirmation.

## 0.2.1 — installation Chrome et désinstallation visible

- Fichier natif `Desinstaller OTM.exe` et raccourci de désinstallation dans le menu Démarrer.
- Parcours Chrome proposé en fin de setup, raccourci dédié et boutons natifs avec le dossier précis à charger.
- Option de packaging `-ChromeWebStoreId` pour la méthode officielle d’installation externe de Chrome sous Windows : registre machine `update_url`, pont machine et retrait à la désinstallation.
- L’application distingue explicitement publication Web Store, installation locale guidée et enregistrement du pont.

## 0.2.0 — migration desktop Rust

- Dépôt recentré sur Rust, moteur et interface egui conservés ; versions Python archivées hors du dépôt.
- Presse-papiers natif eframe activé, focus initial sur le lien et sélection du contenu.
- Invalidation immédiate des anciennes sondes lors d’un changement d’URL.
- Outils Windows sans console, pont navigateur directement sur l’exécutable, suppression du polling `reg.exe` dans l’interface.
- Enregistrement automatique du pont en arrière-plan au démarrage, parcours navigateur explicite.
- Setup Windows par utilisateur, raccourcis, désinstallation et extensions embarquées ; construction sans Python.
- Fichiers partiels distincts par transfert, migration des anciens partiels non ambigus et finalisation des noms sérialisée.
- Progression validée après écriture disque ; erreurs terminales des workers propagées au coordinateur.
- 19 tests automatisés et CI Windows avec formatage, Clippy et compilation du setup.

Les extensions ne sont pas encore publiées/signées sur les stores ; leur activation
reste une action confirmée dans le navigateur. yt-dlp et ffmpeg restent optionnels.
