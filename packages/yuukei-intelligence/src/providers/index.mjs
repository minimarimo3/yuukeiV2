import {
  generateWithGemini,
  interpretWithGemini,
  summarizeMemoryIndexWithGemini
} from "./gemini.mjs";
import {
  generateWithOpenAiCompatible,
  interpretWithOpenAiCompatible,
  summarizeMemoryIndexWithOpenAiCompatible
} from "./openai-compatible.mjs";

export const providers = {
  gemini: {
    generate: generateWithGemini,
    interpret: interpretWithGemini,
    summarizeMemoryIndex: summarizeMemoryIndexWithGemini
  },
  "openai-compatible": {
    generate: generateWithOpenAiCompatible,
    interpret: interpretWithOpenAiCompatible,
    summarizeMemoryIndex: summarizeMemoryIndexWithOpenAiCompatible
  }
};

export async function generateWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider?.generate) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { speak: false }, metadata: { provider: config.provider } };
  }
  return provider.generate(input, config);
}

export async function interpretWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider?.interpret) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { choice: "不明" }, metadata: { provider: config.provider } };
  }
  return provider.interpret(input, config);
}

export async function summarizeMemoryIndexWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider?.summarizeMemoryIndex) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { indexed: false }, metadata: { provider: config.provider } };
  }
  return provider.summarizeMemoryIndex(input, config);
}
