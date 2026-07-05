import assert from "node:assert/strict";
import { once } from "node:events";
import test from "node:test";
import { generateWithGemini } from "../src/providers/gemini.mjs";
import { generateWithOpenAiCompatible } from "../src/providers/openai-compatible.mjs";
import { capabilityResult, parseJsonOutput } from "../src/output.mjs";

const sampleInput = {
  event: {
    type: "conversation.text",
    payload: { text: "こんにちは" }
  },
  persona: {
    actorId: "yuukei",
    displayName: "Yuukei",
    profile: { role: "UI resident", speechStyle: "short" }
  },
  recentContext: [
    {
      kind: "conversation.text",
      timestamp: "2026-01-01T00:00:00.000Z",
      payload: { text: "こんにちは" }
    }
  ],
  constraints: { maxLength: 20 }
};

test("openai-compatible formats request and maps JSON response", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      choices: [
        {
          message: {
            content: JSON.stringify({
              speak: true,
              text: "いるよ。",
              expression: "smile",
              motion: "nod"
            })
          }
        }
      ]
    });
  };
  try {
    const result = await generateWithOpenAiCompatible(sampleInput, {
      timeoutMs: 1000,
      openaiCompatible: {
        baseUrl: "http://127.0.0.1:1234/v1",
        apiKey: "secret",
        model: "local-test"
      }
    });

    assert.deepEqual(result.output, {
      speak: true,
      text: "いるよ。",
      expression: "smile",
      motion: "nod"
    });
    assert.equal(calls[0].url, "http://127.0.0.1:1234/v1/chat/completions");
    assert.equal(calls[0].init.method, "POST");
    assert.equal(calls[0].init.headers.authorization, "Bearer secret");
    assert.equal(calls[0].body.model, "local-test");
    assert.equal(calls[0].body.response_format, undefined);
    assert.equal(calls[0].body.messages[0].role, "system");
    assert.equal(calls[0].body.messages[1].role, "user");
    assert.match(calls[0].body.messages[1].content, /Yuukei/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("openai-compatible can request json_schema response format", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      choices: [{ message: { content: JSON.stringify({ speak: true, text: "schema ok" }) } }]
    });
  };
  try {
    const result = await generateWithOpenAiCompatible(sampleInput, {
      timeoutMs: 1000,
      openaiCompatible: {
        baseUrl: "http://127.0.0.1:1234/v1",
        model: "local-test",
        responseFormat: "json_schema"
      }
    });

    assert.deepEqual(result.output, { speak: true, text: "schema ok" });
    assert.equal(calls[0].body.response_format.type, "json_schema");
    assert.equal(calls[0].body.response_format.json_schema.name, "dialogue_generate_output");
    assert.equal(calls[0].body.response_format.json_schema.strict, true);
    assert.equal(calls[0].body.response_format.json_schema.schema.required[0], "speak");
    assert.equal(
      calls[0].body.response_format.json_schema.schema.additionalProperties,
      false
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("openai-compatible retries once without response_format when json_schema is rejected", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    if (calls.length === 1) {
      return response(400, {
        error: "'response_format.type' must be 'json_schema' or 'text'"
      });
    }
    return response(200, {
      choices: [
        {
          message: {
            content: "```json\n{\"speak\":true,\"text\":\"再送で話せた。\"}\n```"
          }
        }
      ]
    });
  };
  try {
    const result = await generateWithOpenAiCompatible(sampleInput, {
      timeoutMs: 1000,
      openaiCompatible: {
        baseUrl: "http://127.0.0.1:1234/v1",
        model: "local-test",
        responseFormat: "json_schema"
      }
    });

    assert.deepEqual(result.output, { speak: true, text: "再送で話せた。" });
    assert.equal(calls.length, 2);
    assert.equal(calls[0].body.response_format.type, "json_schema");
    assert.equal(calls[1].body.response_format, undefined);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("openai-compatible returns speak false for API errors, timeout, and invalid JSON", async (t) => {
  await t.test("API error", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () => response(500, { error: "nope" });
    try {
      const result = await generateWithOpenAiCompatible(sampleInput, {
        timeoutMs: 1000,
        openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "local-test" }
      });
      assert.deepEqual(result.output, { speak: false });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("timeout", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (_url, init) => {
      await once(init.signal, "abort");
      throw new Error("aborted");
    };
    try {
      const result = await generateWithOpenAiCompatible(sampleInput, {
        timeoutMs: 10,
        openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "local-test" }
      });
      assert.deepEqual(result.output, { speak: false });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("invalid JSON content", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () =>
      response(200, { choices: [{ message: { content: "not json" } }] });
    try {
      const result = await generateWithOpenAiCompatible(sampleInput, {
        timeoutMs: 1000,
        openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "local-test" }
      });
      assert.deepEqual(result.output, { speak: false });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("missing model", async () => {
    const result = await generateWithOpenAiCompatible(sampleInput, {
      timeoutMs: 1000,
      openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "" }
    });
    assert.deepEqual(result.output, { speak: false });
  });
});

test("gemini formats request and maps JSON response", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      candidates: [
        {
          content: {
            parts: [
              {
                text: JSON.stringify({ speak: true, text: "うん、ここにいる。" })
              }
            ]
          }
        }
      ]
    });
  };
  try {
    const result = await generateWithGemini(sampleInput, {
      timeoutMs: 1000,
      gemini: { apiKey: "gem-key", model: "gemini-test" }
    });

    assert.deepEqual(result.output, { speak: true, text: "うん、ここにいる。" });
    assert.equal(
      calls[0].url,
      "https://generativelanguage.googleapis.com/v1beta/models/gemini-test:generateContent?key=gem-key"
    );
    assert.equal(calls[0].init.method, "POST");
    assert.equal(calls[0].init.headers["content-type"], "application/json");
    assert.equal(calls[0].body.generationConfig.responseMimeType, "application/json");
    assert.equal(calls[0].body.generationConfig.responseSchema.properties.speak.type, "BOOLEAN");
    assert.match(calls[0].body.contents[0].parts[0].text, /Yuukei/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("gemini returns speak false for missing key, API errors, timeout, and invalid JSON", async (t) => {
  await t.test("missing key", async () => {
    const result = await generateWithGemini(sampleInput, {
      timeoutMs: 1000,
      gemini: { model: "gemini-test" }
    });
    assert.deepEqual(result.output, { speak: false });
  });

  await t.test("API error", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () => response(500, { error: "nope" });
    try {
      const result = await generateWithGemini(sampleInput, {
        timeoutMs: 1000,
        gemini: { apiKey: "gem-key", model: "gemini-test" }
      });
      assert.deepEqual(result.output, { speak: false });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("timeout", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (_url, init) => {
      await once(init.signal, "abort");
      throw new Error("aborted");
    };
    try {
      const result = await generateWithGemini(sampleInput, {
        timeoutMs: 10,
        gemini: { apiKey: "gem-key", model: "gemini-test" }
      });
      assert.deepEqual(result.output, { speak: false });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("invalid JSON content", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () =>
      response(200, { candidates: [{ content: { parts: [{ text: "no json" }] } }] });
    try {
      const result = await generateWithGemini(sampleInput, {
        timeoutMs: 1000,
        gemini: { apiKey: "gem-key", model: "gemini-test" }
      });
      assert.deepEqual(result.output, { speak: false });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});

test("capability result preserves invocation envelope", () => {
  const result = capabilityResult(
    { id: "cap_1", capability: "dialogue.generate", input: sampleInput },
    { speak: true, text: "長い長い長い長い長い長い長い長い長い長い長い長い" },
    { provider: "test" }
  );
  assert.equal(result.invocationId, "cap_1");
  assert.equal(result.extensionId, "yuukei-intelligence");
  assert.equal(result.capability, "dialogue.generate");
  assert.equal(result.metadata.provider, "test");
  assert.equal(result.output.text.length, sampleInput.constraints.maxLength);
});

test("parseJsonOutput accepts fenced and embedded JSON but rejects non-JSON text", () => {
  assert.deepEqual(parseJsonOutput("```json\n{\"speak\":true,\"text\":\"フェンス\"}\n```", 20), {
    speak: true,
    text: "フェンス"
  });
  assert.deepEqual(parseJsonOutput("前置き {\"speak\":true,\"text\":\"抽出\"} 後置き", 20), {
    speak: true,
    text: "抽出"
  });
  assert.deepEqual(parseJsonOutput("話すかもしれません", 20), { speak: false });
});

function response(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body
  };
}
