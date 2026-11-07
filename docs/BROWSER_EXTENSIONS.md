# Intégration navigateur

Le pont natif et l’extension sont deux éléments différents.

* Le setup copie les extensions à côté de `otm.exe` et enregistre automatiquement
  `org.opentransfermanager.host` dans HKCU pour Chrome, Edge, Brave et Firefox.
* Le manifeste du pont pointe directement sur `otm.exe` : pas de fichier BAT ni de console.
* L’application vérifie les manifestes locaux sans lancer `reg.exe` périodiquement.
* Les mentions « bridge registered » décrivent le pont, pas une connexion active de l’extension.

## Version locale / développement

Dans l’application : Settings → Browsers → « Installer / configurer Chrome », ou le raccourci du
menu Démarrer. Le setup propose également ce parcours à sa dernière étape.
Le bouton « Copier le dossier Chrome » fournit le chemin exact du dossier à charger.

Chrome, Edge et Brave : ouvrir `chrome://extensions`, `edge://extensions` ou
`brave://extensions`, activer le mode développeur, puis « Charger l’extension non empaquetée »
et sélectionner le dossier `chromium`. Le manifeste contient une clé publique fixe pour
stabiliser l’identifiant ; la clé privée reste hors du dépôt.

## Setup avec extension publiée sur le Chrome Web Store

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-windows.ps1 -ChromeWebStoreId IDENTIFIANT_PUBLIC
```

L’identifiant doit correspondre à votre extension effectivement publiée (32 lettres a–p).
Le setup devient alors une installation administrateur dans Program Files. Il écrit
`HKLM\Software\Wow6432Node\Google\Chrome\Extensions\ID\update_url` avec la valeur officielle
`https://clients2.google.com/service/update2/crx`, et enregistre le pont natif machine.
Chrome détecte cette demande à son démarrage ; l’utilisateur confirme l’activation.
Le désinstalleur retire la demande externe et le pont machine.

Sans cet identifiant, le setup reste par utilisateur et propose une installation locale
guidée ; aucune clé d’installation automatique factice n’est créée. Il n’utilise pas les
politiques de force-installation ni ne modifie les préférences ou profils Chrome.

Firefox : ouvrir `about:debugging#/runtime/this-firefox`, charger temporairement le
`manifest.json` du dossier `firefox`. Cette installation de développement disparaît
au redémarrage du navigateur. Une installation permanente nécessite un XPI signé.

## Distribution publique

Chrome sous Windows impose une distribution via Chrome Web Store hors environnement
administré. Firefox impose une signature pour les versions publiques usuelles.
Publier les extensions, vérifier les identifiants dans `src/browsers.rs`, puis proposer
les liens officiels dans l’application. Aucun lien de store inexistant n’est inventé ici.
L’activation finale appartient à l’utilisateur dans le navigateur ; le setup ne contourne
pas les protections en modifiant les profils ou les politiques d’entreprise.

Sources officielles :
- https://developer.chrome.com/docs/extensions/how-to/distribute
- https://developer.chrome.com/docs/extensions/how-to/distribute/install-extensions
- https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/

Le navigateur communique en Native Messaging (stdio encadré), puis le pont contacte
l’API authentifiée sur 127.0.0.1. Les cookies envoyés au moteur ne sont pas imprimés dans les logs.
