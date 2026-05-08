import "../components/iam/TachyonMfaPrompt";
import { connectionStore } from "../stores/connectionStore";

type MfaPromptElement = HTMLElement & {
  open: () => Promise<void>;
};

const sudoGraceMs = 20 * 60 * 1000;

export async function ensureMfa(): Promise<void> {
  const state = connectionStore.getState();
  if (Date.now() - state.lastMfaTimestamp < sudoGraceMs) {
    return;
  }

  const prompt = ensurePrompt();
  await prompt.open();
  connectionStore.getState().setLastMfaTimestamp(Date.now());
}

function ensurePrompt(): MfaPromptElement {
  let prompt = document.querySelector<MfaPromptElement>("tachyon-mfa-prompt");
  if (!prompt) {
    prompt = document.createElement("tachyon-mfa-prompt") as MfaPromptElement;
    document.body.appendChild(prompt);
  }
  return prompt;
}
