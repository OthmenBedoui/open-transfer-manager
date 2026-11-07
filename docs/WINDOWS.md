# Validation Windows

## Vérifications automatisées

`cargo test --locked` vérifie notamment les transferts multi-connexions, pause/reprise,
serveurs sans Range, débit limité, API et protocole Native Messaging. Les tests HTTP
utilisent un serveur local et un dossier temporaire, pas les téléchargements personnels.

`scripts/install-windows.ps1 -BuildOnly` doit réussir puis produire un setup.
La commande normale lance aussi son assistant. Le setup est x64 et par utilisateur.

## Vérification sur une machine propre

1. Installer avec le setup, vérifier le menu Démarrer et la désinstallation Windows.
2. Ouvrir New download, coller avec Ctrl+V sans cliquer ailleurs ; modifier le lien
   pendant qu’une sonde est en cours et vérifier que l’ancien résultat ne remplace pas le nouveau.
3. Laisser l’application ouverte au moins 20 secondes : aucune console `reg.exe` ne doit apparaître.
4. Charger l’extension de développement, puis envoyer un lien ; tester le démarrage
   de l’application fermée et les messages aller/retour du pont.
5. Télécharger deux fichiers homonymes en parallèle ; tester pause/reprise et annulation.
6. Désinstaller : pont retiré, téléchargements et préférences conservés.

La validation automatisée ne remplace pas ces essais de fenêtre et d’installation dans
une VM Windows propre. Les extensions locales ne sont pas des extensions publiées/signées.

## Retest de la version 0.2.1

21 tests réussis (20 pour l’application, 1 pour le lanceur de désinstallation).
Le script `scripts/test-windows-setup.ps1 -IsccPath C:\chemin\ISCC.exe` compile un
setup avec un AppId de test distinct afin de préserver l’installation OTM réelle.
Il vérifie le contenu installé, l’identité Chrome dérivée de la clé publique, les
échanges Native Messaging, puis lance `Desinstaller OTM.exe` et vérifie la suppression
des exécutables, du pont et de l’inscription Windows de test. Les anciennes valeurs
du registre navigateur sont restaurées en fin de test, même en cas d’erreur.

Cette séquence a réussi localement. Le mode de packaging Web Store a été compilé
séparément, sans installer de demande fictive d’extension. L’activation réelle dans
Chrome n’a pas été contrôlée : l’outil de contrôle navigateur a interrompu l’accès
car il ne pouvait pas identifier l’URL courante. Aucun profil Chrome n’a été modifié
pour contourner cette interruption.

## Version 0.2.2 : arabe et confirmation

- 26 tests Rust (25 application + 1 désinstalleur) : glyphes arabes, noms mixtes avec
  chiffres et suffixe latin, sauvegarde sur disque d’un nom arabe, demande API sans
  création de téléchargement, annulation du dialogue et en-têtes liés à l’adresse d’origine.
- Contrôle visuel natif du nom `تقرير العمل 2026.pdf` dans la liste et de son aperçu
  dans la fenêtre « Confirmer le téléchargement ».
- Setup avec licence Noto embarquée et script d’installation/désinstallation isolées.
- Après mise à jour, fermer l’ancienne application et recharger l’extension dans
  `chrome://extensions` pour récupérer les messages « confirmation demandée ».
- Les liens navigateur sont bloqués si l’instance en cours est ancienne ou headless
  et ne déclare pas la capacité de confirmation. Les commandes CLI explicites restent directes.

## Version 0.2.3 : option dorée

- 30 tests réussis : disponibilité conditionnelle du format, maintien de la confirmation
  ordinaire, démarrage direct du choix doré, téléchargement d’une pochette depuis un
  serveur HTTP local et vérification des cadres ID3 après écriture avec paroles Unicode.
- Le test d’intégration ID3 utilise un échantillon audio synthétique et une image de test ;
  il ne constitue pas un test de téléchargement d’une chanson depuis un service tiers.
- Vérification visuelle du nom et du fond dorés dans la fenêtre native.
- Installation/désinstallation isolées et protocole Native Messaging retestés.
- yt-dlp et ffmpeg restent nécessaires. Sans paroles explicites et image compatibles
  fournies par la source, aucun format doré n’est proposé.

## Vérification locale du 7 novembre 2026

- 19 tests Rust réussis, dont collage dans le dialogue egui et invalidation des anciennes sondes.
- `cargo fmt --check` et Clippy avec `-D warnings` réussis.
- Setup Inno Setup compilé ; installation et désinstallation silencieuses testées dans un
  dossier isolé, données isolées avec `OTM_DATA_DIR`, valeurs du registre navigateur restaurées.
- Aller-retour Native Messaging vérifié avec `native-host` et avec l’argument d’origine Chromium.
- Fenêtre egui lancée depuis l’installation de test, capture native générée et inspectée.
- Ce test a lieu sur la machine de développement, pas dans une VM vierge ; les extensions
  n’ont pas été publiées ni installées dans les profils personnels des navigateurs.

## Version 0.2.4 : High Audio enrichi

- Le MP3 High 320 kb/s individuel déclenche la recherche LRCLIB, puis MusicBrainz / Cover Art Archive.
- Les autres qualités, les fichiers originaux, la vidéo et les playlists ne déclenchent pas cette recherche.
- Une absence de paroles ou de pochette ne fait pas échouer l'audio. Le résultat apparaît dans la liste ; le doré indique que les deux tags ont été intégrés.
- 30 tests locaux passent, plus deux vérifications réseau exécutées explicitement : paroles de Get Lucky (Daft Punk) via LRCLIB et décodage d'une vraie pochette de Random Access Memories via Cover Art Archive.
- MusicBrainz a renvoyé une indisponibilité temporaire pendant la vérification ; le repli vers l'image de la source est donc conservé. Les deux tests réseau ne constituent pas un téléchargement complet de chanson depuis YouTube.
- Pour relancer les appels réels : `cargo test live_ -- --ignored --nocapture`.
