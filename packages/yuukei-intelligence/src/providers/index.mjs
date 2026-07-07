import {
  generateWithGemini,
  extractWithGemini,
  interpretWithGemini,
  evaluateMoodWithGemini,
  summarizeMemoryIndexWithGemini
} from "./gemini.mjs";
import {
  evaluateMoodWithOpenAiCompatible,
  extractWithOpenAiCompatible,
  generateWithOpenAiCompatible,
  interpretWithOpenAiCompatible,
  summarizeMemoryIndexWithOpenAiCompatible
} from "./openai-compatible.mjs";

export const providers = {
  gemini: {
    generate: generateWithGemini,
    extract: extractWithGemini,
    interpret: interpretWithGemini,
    evaluateMood: evaluateMoodWithGemini,
    summarizeMemoryIndex: summarizeMemoryIndexWithGemini
  },
  "openai-compatible": {
    generate: generateWithOpenAiCompatible,
    extract: extractWithOpenAiCompatible,
    interpret: interpretWithOpenAiCompatible,
    evaluateMood: evaluateMoodWithOpenAiCompatible,
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

export async function evaluateMoodWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider?.evaluateMood) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { mood: "ふつう", talkDesire: 50, topic: "" }, metadata: { provider: config.provider } };
  }
  return provider.evaluateMood(input, config);
}

export async function interpretWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider?.interpret) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { choice: "不明" }, metadata: { provider: config.provider } };
  }
  return provider.interpret(input, config);
}

export async function extractWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider?.extract) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { found: false, value: "不明" }, metadata: { provider: config.provider } };
  }
  return provider.extract(input, config);
}

export async function summarizeMemoryIndexWithProvider(input, config) {
  const provider = providers[config.provider];
  if (!provider?.summarizeMemoryIndex) {
    console.error(`yuukei-intelligence: unknown provider: ${config.provider}`);
    return { output: { indexed: false }, metadata: { provider: config.provider } };
  }
  return provider.summarizeMemoryIndex(input, config);
}
