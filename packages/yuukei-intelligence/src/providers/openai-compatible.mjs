import { buildDialoguePrompt, buildSystemPrompt } from "../prompt.mjs";
import { parseJsonOutput, silentOutput } from "../output.mjs";

export async function generateWithOpenAiCompatible(input, config) {
  const providerConfig = config.openaiCompatible ?? {};
  const baseUrl = trimTrailingSlash(providerConfig.baseUrl ?? "http://127.0.0.1:1234/v1");
  const model = providerConfig.model ?? "local-model";
  if (!baseUrl || !model) {
    console.error("yuukei-intelligence: openai-compatible baseUrl or model is not configured");
    return { output: silentOutput(), metadata: { provider: "openai-compatible", model } };
  }

  const headers = { "content-type": "application/json" };
  if (providerConfig.apiKey) {
    headers.authorization = `Bearer ${providerConfig.apiKey}`;
  }
  const body = {
    model,
    messages: [
      { role: "system", content: buildSystemPrompt() },
      { role: "user", content: buildDialoguePrompt(input) }
    ],
    temperature: 0.7,
    response_format: { type: "json_object" }
  };

  try {
    const response = await fetchWithTimeout(
      `${baseUrl}/chat/completions`,
      {
        method: "POST",
        headers,
        body: JSON.stringify(body)
      },
      config.timeoutMs
    );
    if (!response.ok) {
      console.error(`yuukei-intelligence: openai-compatible API error ${response.status}`);
      return { output: silentOutput(), metadata: { provider: "openai-compatible", model } };
    }
    const json = await response.json();
    const text = json?.choices?.[0]?.message?.content;
    return {
      output: parseJsonOutput(text, input?.constraints?.maxLength),
      metadata: { provider: "openai-compatible", model }
    };
  } catch (error) {
    console.error(`yuukei-intelligence: openai-compatible request failed: ${error.message}`);
    return { output: silentOutput(), metadata: { provider: "openai-compatible", model } };
  }
}

function trimTrailingSlash(value) {
  return String(value).replace(/\/+$/, "");
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
