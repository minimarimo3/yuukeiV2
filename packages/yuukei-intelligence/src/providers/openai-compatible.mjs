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
    temperature: 0.7
  };
  if (providerConfig.responseFormat === "json_schema") {
    body.response_format = dialogueGenerateResponseFormat();
  }

  try {
    const url = `${baseUrl}/chat/completions`;
    let response = await postChatCompletion(url, headers, body, config.timeoutMs);
    if (!response.ok && body.response_format) {
      console.error(
        `yuukei-intelligence: openai-compatible API error ${response.status}; retrying without response_format`
      );
      const fallbackBody = { ...body };
      delete fallbackBody.response_format;
      response = await postChatCompletion(url, headers, fallbackBody, config.timeoutMs);
    }
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

function dialogueGenerateResponseFormat() {
  return {
    type: "json_schema",
    json_schema: {
      name: "dialogue_generate_output",
      strict: true,
      schema: {
        type: "object",
        additionalProperties: false,
        properties: {
          speak: { type: "boolean" },
          text: { type: "string" },
          expression: { type: "string" },
          motion: { type: "string" }
        },
        required: ["speak"]
      }
    }
  };
}

function trimTrailingSlash(value) {
  return String(value).replace(/\/+$/, "");
}

async function postChatCompletion(url, headers, body, timeoutMs) {
  return await fetchWithTimeout(
    url,
    {
      method: "POST",
      headers,
      body: JSON.stringify(body)
    },
    timeoutMs
  );
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
