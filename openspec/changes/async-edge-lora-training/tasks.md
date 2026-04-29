# Tasks: Async Edge Training Implementation

**Agent Instruction:** Implémentez la boucle d'entraînement asynchrone en isolant strictement ce processus des workers temps-réel.

- [x] **Interface WIT :** Créer `wit/ai/training.wit` avec la méthode `submit-job`.
- [ ] **Background Worker :** Créer un thread Tokio séparé dans `core-host` pour la queue "low-priority" gérée par `system-faas-buffer`.
- [ ] **Intégration Candle Autograd :** Implémenter la logique d'entraînement LoRA en Rust (avec Candle) avec fallback CPU/RAM.
- [ ] **Data Pipeline :** Câbler la lecture du dataset local via `embedded-core-store`.
- [ ] **Export & Broker :** Sérialiser les poids finaux en `.safetensors` et l'enregistrer dans le `system-faas-model-broker`.
- [ ] **FinOps :** Mesurer la consommation CPU/RAM pour l'imputation Multi-Tenant.
