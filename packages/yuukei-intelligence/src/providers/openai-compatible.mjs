import {
  buildDialoguePrompt,
  buildInterpretPrompt,
  buildInterpretSystemPrompt,
  buildMemoryIndexPrompt,
  buildMemoryIndexSystemPrompt,
  buildSystemPrompt
} from "../prompt.mjs";
import {
  parseJsonInterpretOutput,
  parseJsonMemoryIndexOutput,
  parseJsonOutput,
  memoryIndexFailureOutput,
  silentOutput,
  unknownChoiceOutput
} from "../output.mjs";

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
      metadata: openAiCompatibleMetadata(json, model)
    };
  } catch (error) {
    console.error(`yuukei-intelligence: openai-compatible request failed: ${error.message}`);
    return { output: silentOutput(), metadata: { provider: "openai-compatible", model } };
  }
}

export async function interpretWithOpenAiCompatible(input, config) {
  const providerConfig = config.openaiCompatible ?? {};
  const baseUrl = trimTrailingSlash(providerConfig.baseUrl ?? "http://127.0.0.1:1234/v1");
  const model = providerConfig.model ?? "local-model";
  if (!baseUrl || !model) {
    console.error("yuukei-intelligence: openai-compatible baseUrl or model is not configured");
    return { output: unknownChoiceOutput(), metadata: { provider: "openai-compatible", model } };
  }

  const headers = { "content-type": "application/json" };
  if (providerConfig.apiKey) {
    headers.authorization = `Bearer ${providerConfig.apiKey}`;
  }
  const body = {
    model,
    messages: [
      { role: "system", content: buildInterpretSystemPrompt() },
      { role: "user", content: buildInterpretPrompt(input) }
    ],
    temperature: 0.1
  };
  if (providerConfig.responseFormat === "json_schema") {
    body.response_format = dialogueInterpretResponseFormat();
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
      return { output: unknownChoiceOutput(), metadata: { provider: "openai-compatible", model } };
    }
    const json = await response.json();
    const text = json?.choices?.[0]?.message?.content;
    return {
      output: parseJsonInterpretOutput(text, input?.choices),
      metadata: openAiCompatibleMetadata(json, model)
    };
  } catch (error) {
    console.error(`yuukei-intelligence: openai-compatible request failed: ${error.message}`);
    return { output: unknownChoiceOutput(), metadata: { provider: "openai-compatible", model } };
  }
}

export async function summarizeMemoryIndexWithOpenAiCompatible(input, config) {
  const providerConfig = config.openaiCompatible ?? {};
  const baseUrl = trimTrailingSlash(providerConfig.baseUrl ?? "http://127.0.0.1:1234/v1");
  const model = providerConfig.model ?? "local-model";
  if (!baseUrl || !model) {
    console.error("yuukei-intelligence: openai-compatible baseUrl or model is not configured");
    return { output: memoryIndexFailureOutput(), metadata: { provider: "openai-compatible", model } };
  }

  const headers = { "content-type": "application/json" };
  if (providerConfig.apiKey) {
    headers.authorization = `Bearer ${providerConfig.apiKey}`;
  }
  const body = {
    model,
    messages: [
      { role: "system", content: buildMemoryIndexSystemPrompt() },
      { role: "user", content: buildMemoryIndexPrompt(input) }
    ],
    temperature: 0.3
  };

  try {
    const response = await postChatCompletion(
      `${baseUrl}/chat/completions`,
      headers,
      body,
      config.timeoutMs
    );
    if (!response.ok) {
      console.error(`yuukei-intelligence: openai-compatible API error ${response.status}`);
      return { output: memoryIndexFailureOutput(), metadata: { provider: "openai-compatible", model } };
    }
    const json = await response.json();
    const text = json?.choices?.[0]?.message?.content;
    const output = parseJsonMemoryIndexOutput(text);
    return {
      output: output ?? memoryIndexFailureOutput(),
      metadata: openAiCompatibleMetadata(json, model)
    };
  } catch (error) {
    console.error(`yuukei-intelligence: openai-compatible request failed: ${error.message}`);
    return { output: memoryIndexFailureOutput(), metadata: { provider: "openai-compatible", model } };
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

function dialogueInterpretResponseFormat() {
  return {
    type: "json_schema",
    json_schema: {
      name: "dialogue_interpret_output",
      strict: true,
      schema: {
        type: "object",
        additionalProperties: false,
        properties: {
          choice: { type: "string" },
          confidence: { type: "number" }
        },
        required: ["choice"]
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

function openAiCompatibleMetadata(json, model) {
  const metadata = { provider: "openai-compatible", model };
  const inputTokens = json?.usage?.prompt_tokens;
  const outputTokens = json?.usage?.completion_tokens;
  if (Number.isFinite(inputTokens) && Number.isFinite(outputTokens)) {
    metadata.usage = {
      inputTokens,
      outputTokens,
      model,
      provider: "openai-compatible"
    };
  }
  return metadata;
}
