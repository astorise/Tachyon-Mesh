## Context

`core-host` exécute aujourd'hui tous les invités WebAssembly comme des modules WASI preview1 qui consomment `stdin` et écrivent leur réponse sur `stdout`. Ce contrat a permis d'introduire rapidement des invités Rust, Go, JavaScript, C# et Java, mais il force la sérialisation implicite des requêtes HTTP et ne permet pas d'exprimer une interface typée entre l'hôte et le guest.

Le changement `component-model-wit` doit introduire le WebAssembly Component Model pour `guest-example` sans casser les invités WASI existants déjà couverts par `polyglot-faas`, `http-routing` et `k3d-integration-test`.

## Goals / Non-Goals

**Goals:**
- Définir un contrat WIT partagé pour l'échange `Request`/`Response`.
- Compiler `guest-example` comme composant WebAssembly `wasm32-wasip2`.
- Faire préférer à `core-host` l'exécution Component Model quand l'artefact le permet.
- Conserver un fallback WASI preview1 pour les invités non migrés.
- Mettre à jour la chaîne de build locale, CI et Docker pour produire l'artefact composant.

**Non-Goals:**
- Migrer les invités Go, JavaScript, C#, Java ou `guest-call-legacy` au Component Model.
- Introduire un contrat WIT pour les appels mesh sortants ou l'observabilité guest.
- Refondre `integrity.lock`, les manifests Kubernetes ou la topologie réseau.

## Decisions

### 1. Ajouter un monde WIT minimal au niveau workspace

Le contrat `wit/tachyon.wit` expose une interface `handler` avec deux records (`request`, `response`) et une fonction `handle-request`.

Pourquoi:
- couvre le besoin immédiat d'un appel HTTP typé;
- reste assez simple pour être réutilisé par d'autres invités Rust;
- évite d'introduire des dépendances WIT externes supplémentaires.

Alternative rejetée:
- Utiliser une interface plus riche avec headers et metadata dès maintenant. C'est utile à terme, mais inutilement coûteux pour cette migration initiale.

### 2. Compiler uniquement `guest-example` en `wasm32-wasip2`

`guest-example` devient le premier invité composant et sert de chemin de référence. Les autres invités continuent à sortir des modules WASI preview1.

Pourquoi:
- limite le rayon d'impact;
- conserve la compatibilité avec les invités polyglottes existants;
- permet d'introduire le Component Model sans bloquer le reste du dépôt.

Alternative rejetée:
- Basculer tous les invités simultanément. Cela casserait `polyglot-faas` et la chaîne Docker actuelle pour des gains limités.

### 3. Préférer le composant, puis retomber sur l'exécution WASI existante

`core-host` résout toujours le même nom d'artefact, tente d'abord `wasmtime::component::Component`, puis utilise le pipeline `Module + WASI preview1` si le fichier n'est pas un composant valide.

Pourquoi:
- garde un point d'entrée unique pour les routes existantes;
- évite de multiplier les conventions de nommage;
- protège les capacités déjà archivées.

Alternative rejetée:
- Introduire deux résolveurs ou deux extensions de fichier distinctes. Cela rendrait la migration plus lourde sans réel bénéfice.

### 4. Préserver les réponses HTTP existantes de `guest-example`

Le composant retourne toujours `FaaS received: <payload>` pour un corps non vide et `FaaS received an empty payload` sinon.

Pourquoi:
- maintient les tests HTTP et l'intégration k3d existants;
- concentre ce changement sur la frontière d'exécution, pas sur le comportement métier.

## Risks / Trade-offs

- [Compatibilité Wasmtime / bindgen] -> Utiliser un contrat WIT minimal et verrouiller le comportement avec des tests host.
- [Régression sur les invités non migrés] -> Garder le chemin WASI preview1 intact et le couvrir par un test de fallback.
- [Écart entre build local et CI] -> Mettre à jour à la fois `Dockerfile`, `ci.yml` et la documentation de build.
- [Perte de visibilité guest via stdout] -> Limiter la migration composant à `guest-example`; les invités observables existants restent compatibles.

## Migration Plan

1. Ajouter le contrat WIT partagé.
2. Migrer `guest-example` vers `wit-bindgen` et `wasm32-wasip2`.
3. Ajouter les bindings Component Model et le fallback legacy dans `core-host`.
4. Mettre à jour les builds CI/Docker et vérifier les réponses HTTP inchangées.
5. Archiver le changement avec synchronisation des specs.

## Open Questions

- Aucun blocage ouvert pour cette itération. Les headers HTTP et les interfaces mesh sortantes restent volontairement hors scope.
