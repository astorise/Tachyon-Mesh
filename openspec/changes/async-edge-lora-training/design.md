# Design: Async Edge Training Architecture

## 1. Wasm Interface (Fire-and-Forget)
L'interface `tachyon:ai/training` permet de soumettre un job. L'instance FaaS Wasm soumet la tâche et se termine immédiatement, préservant la latence sub-milliseconde des flux HTTP/3.

## 2. Low-Priority Message Broker
Le Job ID est poussé dans `system-faas-buffer`. 
- Des workers asynchrones dédiés consomment cette file.
- Le thread d'entraînement est assigné à une priorité OS basse (Nice level) pour ne pas impacter l'inférence temps-réel.

## 3. Gestion Mémoire : VRAM Spillover
L'entraînement nécessite de stocker l'arbre d'autograd. 
- Candle est configuré pour détecter la pression VRAM.
- Les états de l'optimiseur et les gradients sont offloadés en RAM (DDR). 
- Bien que plus lent, cela garantit la complétion du job sans impacter la VRAM réservée aux fonctions FaaS actives.

## 4. Finalisation
Le fichier `.safetensors` généré est stocké localement, haché, et mis à disposition du `Large Model Broker` pour être utilisé par les futurs appels d'inférence du même Tenant.