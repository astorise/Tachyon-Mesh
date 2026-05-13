# Technical Specification: IAM Decomposition

## 1. Component Extraction (`auth-step-credentials`)
Create `tachyon-ui/src/components/iam/TachyonAuthStepCredentials.ts`.
Move the Stronghold unlock, cluster URL selection, and PAT input form here.

```typescript
export class TachyonAuthStepCredentials extends HTMLElement {
  // Handles rendering the initial login form.
  // Emits 'credentials-submitted' event containing the raw inputs.
}
customElements.define('auth-step-credentials', TachyonAuthStepCredentials);
```

## 2. Component Extraction (`auth-step-mfa`)
Ensure `tachyon-ui/src/components/iam/TachyonMfaPrompt.ts` acts exclusively as a dumb presentation component. It should not make network calls directly, but emit an `mfa-submitted` event.

## 3. IAM Orchestration
Refactor `TachyonIAM.ts` to act purely as a state machine switching between the sub-components based on `connectionStore` states.

```typescript
// Pseudocode for TachyonIAM.ts render logic
render() {
  const state = connectionStore.getState();
  if (state.requiresMfa) {
    this.container.innerHTML = `<auth-step-mfa></auth-step-mfa>`;
  } else {
    this.container.innerHTML = `<auth-step-credentials></auth-step-credentials>`;
  }
}
```