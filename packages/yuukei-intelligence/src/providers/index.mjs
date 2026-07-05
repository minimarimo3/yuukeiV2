import { generateWithGemini } from "./gemini.mjs";
import { generateWithOpenAiCompatible } from "./openai-compatible.mjs";

export const providers = {
  gemini: generateWithGemini,
  "openai-compatible": generateWithOpenAiCompatible
};

export async function generateWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { speak: false }, metadata: { provider: config.provider } };
  }
  return provider(input, config);
}
