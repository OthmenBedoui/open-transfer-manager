# Open Transfer Manager

Application desktop native en **Rust + egui/eframe**, sans Python, Electron ou WebView.
Téléchargements HTTP multiconnexions, pause/reprise, files d’attente, planification,
limiteur global et intégration navigateur. Vidéo/audio via yt-dlp et ffmpeg optionnels.

## Windows : compiler et ouvrir le setup

Prérequis de développement : Rust stable, un linker Windows (MSVC Build Tools recommandé,
ou MinGW-w64 pour le toolchain GNU), PowerShell 5.1+ et Inno Setup 6/7.
Inno Setup peut être installé avec `winget install JRSoftware.InnoSetup`.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-windows.ps1
```

La commande lance les tests, compile en release, prépare les extensions et produit
`dist/windows/OTM-Setup-0.2.6.exe`, puis ouvre son assistant d’installation.
Ajouter `-BuildOnly` pour produire le setup sans l’installer, ou `-IsccPath ...` pour
indiquer le compilateur Inno Setup. Les utilisateurs finaux exécutent seulement le setup :
ni Rust, ni Python, ni terminal ne sont requis.

Installation par utilisateur, sans droits administrateur : menu Démarrer, raccourci
bureau optionnel, désinstallation Windows et pont navigateur enregistré automatiquement.
La désinstallation conserve les téléchargements et préférences de l’utilisateur.
Un fichier natif `Desinstaller OTM.exe` est présent dans le dossier installé et un
raccourci de désinstallation est ajouté au menu Démarrer.
Le même fichier peut être utilisé séparément : il retrouve l’installation dans le
registre Windows, sans PowerShell, BAT ou fenêtre terminal.
Le setup local n’est pas signé : une distribution publique nécessite une signature de code.

## Extensions navigateur

Les bundles Chrome/Edge/Brave et Firefox sont inclus dans le setup. **Enregistrer le pont
n’installe pas l’extension dans un profil navigateur.** Les navigateurs demandent leur
propre confirmation. Voir [le parcours navigateur](docs/BROWSER_EXTENSIONS.md).
Il n’y a ni modification forcée des profils ni ajout de politiques d’entreprise.
La dernière étape du setup ouvre le parcours Chrome et les instructions natives.
Sans publication Web Store, charger le dossier `extensions/chromium` via le mode développeur.

Pour une extension **déjà publiée**, le développeur peut compiler avec
`-ChromeWebStoreId IDENTIFIANT_PUBLIC`. Ce setup demande les droits administrateur,
enregistre la demande d’installation Chrome via le registre officiel `update_url`,
et installe aussi le pont natif au niveau machine. Chrome récupère l’extension à son
démarrage et demande confirmation d’activation. Aucun identifiant de publication
inexistant n’est configuré par défaut.

## Développement

```powershell
cargo run
cargo test --locked
cargo clippy --all-targets --locked
cargo fmt --check
```

Le lancement normal répare/enregistre le pont local si nécessaire ; il ne relance pas
l’installateur à chaque ouverture. `cargo run -- --setup-browsers` ouvre les réglages navigateur.
`cargo run -- headless` lance le moteur sans interface. `OTM_DATA_DIR` permet d’isoler
les données pour les essais. Les commandes CLI sont disponibles avec `cargo run -- --help`.

Les fonctions vidéo nécessitent yt-dlp ; son installation est proposée dans les réglages.
ffmpeg est nécessaire pour les conversions MP3 et certains assemblages vidéo.
Pour un setup hors ligne avec ces outils, passer `-MediaToolsDirectory C:\outils` contenant
les exécutables et fournir leurs licences conformément à leur distribution. Par défaut,
ils ne sont pas embarqués afin de garder une installation légère.

## Organisation

| Dossier | Rôle |
| --- | --- |
| `src/model.rs`, `src/manager.rs` | Modèle, persistance, file, planification, limiteur |
| `src/http.rs`, `src/media.rs`, `src/music.rs` | Moteurs HTTP, vidéo/audio et intégration musicale ID3 |
| `src/ui/` | Interface egui, thème, composants et dialogues |
| `src/api.rs`, `src/native_host.rs`, `src/browsers.rs` | API locale et pont navigateur |
| `extension/` | Sources JavaScript/CSS des extensions, pas de page web pour l’application |
| `installer/`, `scripts/` | Setup Windows et préparation des bundles sans Python |
| `assets/`, `docs/` | Identité visuelle, licences et documentation |

Voir [l’architecture](docs/ARCHITECTURE.md) et [la validation Windows](docs/WINDOWS.md).
Les noms de fichiers arabes disposent d’une police Noto Sans Arabic embarquée et
d’un affichage bidirectionnel adapté aux chiffres et suffixes latins. Les noms et
chemins réels restent en Unicode, sans inversion ni translittération. Le champ
d’édition conserve le texte logique ; un aperçu lisible est fourni pour les noms arabes.

Les liens envoyés par le navigateur affichent une confirmation native avant toute
création de téléchargement : Télécharger, Ajouter à la file ou Annuler. Plusieurs
demandes sont présentées successivement. Les en-têtes du navigateur sont conservés
pour le lien d’origine et retirés si l’utilisateur modifie son adresse.
Les commandes CLI explicites `otm add` et les reprises existantes gardent leur fonctionnement.

**Sites de confiance.** La fenêtre de confirmation propose « Toujours autoriser les
téléchargements depuis ce site ». Les liens envoyés ensuite depuis ce site (et ses
sous-domaines) démarrent sans confirmation ; tous les autres sites continuent de demander.
Le site retenu est celui de la page d’origine (Referer), sinon celui du lien. La liste se
gère dans Réglages → Browsers ; elle est vide par défaut.

### Pochette et paroles des MP3 (option, désactivée par défaut)

Réglages → Vidéo & audio → **Add cover art and lyrics to MP3 files**. Une fois activée,
l'option s'applique à tout MP3 individuel, quelle que soit sa qualité (High, Standard ou
Small) : l'application cherche les paroles avec LRCLIB, puis la pochette avec
MusicBrainz / Cover Art Archive. Tant qu'elle reste désactivée, aucun de ces services
n'est contacté. Aucun compte ni clé API n'est nécessaire. Titre, artiste et durée sont comparés avant d'accepter les paroles.
L'image de la source sert de repli lorsque la pochette d'album est indisponible.
Les requêtes utilisent des délais limités et respectent le débit MusicBrainz.

Les tags ID3 `USLT` (paroles) et `APIC` (image) sont intégrés dans une copie vérifiée
avant remplacement. Une recherche manquante ou une panne de service ne bloque pas
le MP3 : le résultat de l'enrichissement apparaît dans sa ligne. La couleur dorée
indique que les deux éléments ont été intégrés. Aucun popup de détection ni format
Gold distinct. L'audio original (sans conversion) et la vidéo ne déclenchent aucune recherche.
L'enrichissement concerne les morceaux individuels, pas les playlists. Les paroles restent
la propriété de leurs auteurs : l'option est réservée à un usage personnel.
La conversion 320 kb/s n'améliore pas la qualité de la source.

Services : https://lrclib.net/docs et https://musicbrainz.org/doc/Cover_Art_Archive/API.
Le titre, l'artiste, la durée et l'album sont envoyés aux services concernés.
Les versions Python et fichiers historiques ont été sortis de ce dépôt dans une archive
locale ; l’historique Git reste conservé. Licence MIT ; polices Inter et Noto Sans Arabic sous OFL.
