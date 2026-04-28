# Proposal: Async Edge LoRA Training (Local Fine-Tuning)

## Context
Historiquement, l'entraînement de modèles d'IA à l'Edge est proscrit à cause de la saturation de la VRAM et du blocage des processus vitaux. Cependant, dans un contexte "Air-Gapped" et "Zero-Trust" strict, l'envoi de données privées vers le cloud est inacceptable. Tachyon Mesh doit permettre la création de fichiers adaptateurs LoRA (`.safetensors`) directement sur le nœud Edge en convertissant cette contrainte en une tâche asynchrone, lente et tolérante aux limitations matérielles.

## Objective
1. Permettre à un FaaS Wasm de soumettre une tâche de fine-tuning (LoRA) via une interface standardisée (`wit/ai/training.wit`).
2. Exécuter cet entraînement en tâche de fond via un Message Broker local (`system-faas-buffer`) en priorité basse.
3. Permettre au moteur Candle d'utiliser la RAM système en débordement de la VRAM (Spillover) pour éviter les crashs OOM.

## Scope
- Création du contrat `wit/ai/training.wit`.
- Ajout d'une file d'attente "Low-Priority" dans `system-faas-buffer`.
- Configuration du moteur Candle pour le fallback CPU/RAM lors de la rétropropagation.
- Sauvegarde du `.safetensors` résultant dans le `system-faas-model-broker`.