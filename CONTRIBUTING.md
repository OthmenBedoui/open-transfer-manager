# Contribuer

Le code de l’application est dans `src/` et l’interface native dans `src/ui/`.
Conserver la séparation entre modèle, moteur et interface. Les extensions utilisent
JavaScript/CSS dans `extension/` ; il n’y a pas de runtime Python.

Avant une proposition de changement :

```text
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Pour Windows, construire aussi le setup avec `scripts/install-windows.ps1 -BuildOnly`.
Pour un correctif moteur, privilégier un serveur HTTP local et des fichiers temporaires
dans les tests. Ne pas utiliser les dossiers personnels ou cookies réels.
Pour l’interface, vérifier clair/sombre, raccourcis clavier et mise à l’échelle Windows.

Ne jamais ajouter `target/`, `dist/`, données personnelles, cookies ou clés de signature
au suivi Git. Les sorties de compilation sont régénérées par les scripts.
