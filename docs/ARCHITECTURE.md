# Architecture

```text
egui desktop ───────────────┐
CLI ───── API loopback ─────┼── Manager ── HTTP workers ── fichiers partiels
extension ─ native host ───┘           └─ yt-dlp / ffmpeg (optionnels)
```

`Manager` partage les téléchargements via `Arc<Mutex<Download>>`. Chaque téléchargement
possède son thread de coordination ; les segments HTTP utilisent plusieurs connexions.
Les tâches réseau et les sondes des dialogues ne s’exécutent pas dans la boucle de dessin.
Les événements demandent un rafraîchissement egui. Le modèle ne dépend pas des widgets.

`Settings` et `Download` sont sérialisés en JSON ; les données transitoires ne le sont pas.
Les fichiers sont sous `%APPDATA%/OpenTransferManager` (ou `OTM_DATA_DIR`). Un identifiant
de transfert distingue les fichiers partiels de téléchargements homonymes.

`paths::background_command` masque les consoles des outils Windows. Le Native Messaging
utilise directement le binaire de l’application et conserve les pipes stdin/stdout.
L’API est limitée au loopback, exige un jeton (sauf ping) et valide l’en-tête Host.

L’interface reste egui/eframe avec backend OpenGL ; aucune page HTML ne constitue l’UI desktop.
Le presse-papiers natif est activé explicitement dans les fonctionnalités eframe.

`ui/text.rs` prépare les séquences bidirectionnelles d’affichage ; HarfRust dans egui
forme les glyphes arabes avec la police Noto embarquée. Le modèle et les noms réels
ne passent jamais par cette transformation d’affichage.

Les requêtes navigateur deviennent des `DownloadRequest` transitoires, portées par
`Event::ConfirmDownload`. L’API répond `pending_confirmation`, sans créer de `Download`.
La fenêtre conserve une file de confirmations et ouvre un seul dialogue à la fois.
Le fichier et son enregistrement sont créés seulement après validation du dialogue.
Une instance headless refuse les demandes nécessitant une interface ; les appels CLI
explicites conservent la route d’ajout direct.

L’installateur est distinct du lancement quotidien : il prépare les raccourcis,
la désinstallation, les bundles d’extensions et l’enregistrement du pont. Les caches de
compilation et paquets générés sont ignorés par Git.

La pochette et les paroles dépendent du réglage `Settings.music_tags` (désactivé par
défaut) : sans lui, `music.rs` n’est jamais appelé et aucun service tiers n’est contacté.
Activé, il s’applique à tout MP3 individuel (`media::wants_music_tags`) ; `MediaOpts.music_enriched`
enregistre seulement le résultat (affichage doré). L’option exige ffmpeg. `music.rs` valide la
pochette, limite sa taille et ses dimensions, puis écrit les métadonnées sur une copie
du MP3 avant remplacement. Le navigateur traite le clic sur cette option comme une
confirmation ; les autres demandes passent toujours par le dialogue existant.
