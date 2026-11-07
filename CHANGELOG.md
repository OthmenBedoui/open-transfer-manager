# Historique

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
