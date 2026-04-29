# Tasks: Async Edge Training Implementation

**Agent Instruction:** Implementez la boucle d'entrainement asynchrone en isolant strictement ce processus des workers temps-reel.

- [x] **Interface WIT :** Creer `wit/ai/training.wit` avec la methode `submit-job`.
- [x] **Background Worker :** Creer un thread Tokio separe dans `core-host` pour la queue "low-priority" geree par `system-faas-buffer`.
- [x] **Integration Candle Autograd :** Implementer la logique d'entrainement LoRA en Rust (avec Candle) avec fallback CPU/RAM.
- [x] **Data Pipeline :** Cabler la lecture du dataset local via `embedded-core-store`.
- [x] **Export & Broker :** Serialiser les poids finaux en `.safetensors` et l'enregistrer dans le `system-faas-model-broker`.
- [x] **FinOps :** Mesurer la consommation CPU/RAM pour l'imputation Multi-Tenant.
