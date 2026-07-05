import { buildDialoguePrompt, buildInterpretPrompt } from "../prompt.mjs";
import {
  normalizeOutput,
  parseJsonInterpretOutput,
  parseJsonOutput,
  silentOutput,
  unknownChoiceOutput
} from "../output.mjs";

const GEMINI_ENDPOINT = "https://generativelanguage.googleapis.com/v1beta";

export async function generateWithGemini(input, config) {
  const providerConfig = config.gemini ?? {};
  const apiKey = providerConfig.apiKey;
  const model = providerConfig.model ?? "gemini-2.5-flash";
  if (!apiKey) {
    console.error("yuukei-intelligence: GEMINI_API_KEY is not configured");
    return { output: silentOutput(), metadata: { provider: "gemini", model } };
  }

  const url = `${GEMINI_ENDPOINT}/models/${encodeURIComponent(model)}:generateContent?key=${encodeURIComponent(apiKey)}`;
  const body = {
    contents: [
      {
        role: "user",
        parts: [{ text: buildDialoguePrompt(input) }]
      }
    ],
    generationConfig: {
      responseMimeType: "application/json",
      responseSchema: dialogueGenerateSchema()
    }
  };

  try {
    const response = await fetchWithTimeout(
      url,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body)
      },
      config.timeoutMs
    );
    if (!response.ok) {
      console.error(`yuukei-intelligence: gemini API error ${response.status}`);
      return { output: silentOutput(), metadata: { provider: "gemini", model } };
    }
    const json = await response.json();
    const text = json?.candidates?.[0]?.content?.parts?.[0]?.text;
    return {
      output: parseJsonOutput(text, input?.constraints?.maxLength),
      metadata: { provider: "gemini", model }
    };
  } catch (error) {
    console.error(`yuukei-intelligence: gemini request failed: ${error.message}`);
    return { output: silentOutput(), metadata: { provider: "gemini", model } };
  }
}

export async function interpretWithGemini(input, config) {
  const providerConfig = config.gemini ?? {};
  const apiKey = providerConfig.apiKey;
  const model = providerConfig.model ?? "gemini-2.5-flash";
  if (!apiKey) {
    console.error("yuukei-intelligence: GEMINI_API_KEY is not configured");
    return { output: unknownChoiceOutput(), metadata: { provider: "gemini", model } };
  }

  const url = `${GEMINI_ENDPOINT}/models/${encodeURIComponent(model)}:generateContent?key=${encodeURIComponent(apiKey)}`;
  const body = {
    contents: [
      {
        role: "user",
        parts: [{ text: buildInterpretPrompt(input) }]
      }
    ],
    generationConfig: {
      temperature: 0.1,
      responseMimeType: "application/json",
      responseSchema: dialogueInterpretSchema()
    }
  };

  try {
    const response = await fetchWithTimeout(
      url,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body)
      },
      config.timeoutMs
    );
    if (!response.ok) {
      console.error(`yuukei-intelligence: gemini API error ${response.status}`);
      return { output: unknownChoiceOutput(), metadata: { provider: "gemini", model } };
    }
    const json = await response.json();
    const text = json?.candidates?.[0]?.content?.parts?.[0]?.text;
    return {
      output: parseJsonInterpretOutput(text, input?.choices),
      metadata: { provider: "gemini", model }
    };
  } catch (error) {
    console.error(`yuukei-intelligence: gemini request failed: ${error.message}`);
    return { output: unknownChoiceOutput(), metadata: { provider: "gemini", model } };
  }
}

function dialogueGenerateSchema() {
  return {
    type: "OBJECT",
    properties: {
      speak: { type: "BOOLEAN" },
      text: { type: "STRING" },
      expression: { type: "STRING" },
      motion: { type: "STRING" }
    },
    required: ["speak"]
  };
}

function dialogueInterpretSchema() {
  return {
    type: "OBJECT",
    properties: {
      choice: { type: "STRING" },
      confidence: { type: "NUMBER" }
    },
    required: ["choice"]
  };
}

async function fetchWithTimeout(url, init, timeoutMs = 10_000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

export function normalizeGeminiOutput(value, maxLength) {
  return normalizeOutput(value, maxLength);
}
