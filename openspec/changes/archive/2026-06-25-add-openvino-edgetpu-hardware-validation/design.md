## Approach

Implement vendor runtimes as optional host backends beside Candle. Availability becomes true only after SDK initialization and physical device discovery. Guest-facing dispatch resolves the preferred class through the configured fallback policy. Acceptance requires labeled hardware evidence.
