import assert from "node:assert/strict";
import { once } from "node:events";
import test from "node:test";
import {
  evaluateMoodWithGemini,
  extractWithGemini,
  generateWithGemini,
  interpretWithGemini
} from "../src/providers/gemini.mjs";
import {
  evaluateMoodWithOpenAiCompatible,
  extractWithOpenAiCompatible,
  generateWithOpenAiCompatible,
  interpretWithOpenAiCompatible
} from "../src/providers/openai-compatible.mjs";
import {
  capabilityResult,
  normalizeMoodEvaluateOutput,
  parseJsonInterpretOutput,
  parseJsonMoodEvaluateOutput,
  parseJsonOutput
} from "../src/output.mjs";

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

const sampleInterpretInput = {
  question: "返事は肯定ですか？",
  choices: ["はい", "いいえ"],
  input: { text: "あ〜うん。いやちょっと忙しくて..." }
};

const sampleExtractInput = {
  instruction: "ユーザーの呼び名",
  input: { text: "ミナって呼んで" }
};

const sampleMoodInput = {
  residentId: "resident-default",
  worldPackId: "default-yuukei",
  currentTime: "2026-07-06T12:00:00.000Z",
  timePeriod: "昼",
  secondsSinceLastUserActivity: 240,
  persona: {
    actorId: "yuukei",
    displayName: "Yuukei",
    profile: { role: "UI resident", speechStyle: "short" }
  },
  recentContext: sampleInput.recentContext
};

test("openai-compatible formats request and maps JSON response", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      usage: {
        prompt_tokens: 12,
        completion_tokens: 5
      },
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
    assert.deepEqual(result.metadata.usage, {
      inputTokens: 12,
      outputTokens: 5,
      model: "local-test",
      provider: "openai-compatible"
    });
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

test("openai-compatible includes Daihon generation instruction in prompt", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      choices: [{ message: { content: JSON.stringify({ speak: true, text: "楽しみだなあ。" }) } }]
    });
  };
  try {
    const result = await generateWithOpenAiCompatible(
      { ...sampleInput, instruction: "お出かけの日の楽しみを一言つぶやく" },
      {
        timeoutMs: 1000,
        openaiCompatible: {
          baseUrl: "http://127.0.0.1:1234/v1",
          model: "local-test"
        }
      }
    );

    assert.deepEqual(result.output, { speak: true, text: "楽しみだなあ。" });
    assert.match(calls[0].body.messages[1].content, /Daihon author instruction/);
    assert.match(calls[0].body.messages[1].content, /お出かけの日の楽しみを一言つぶやく/);
    assert.match(calls[0].body.messages[1].content, /one short line/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("openai-compatible includes retrieved memories in prompt", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      choices: [{ message: { content: JSON.stringify({ speak: true, text: "覚えてるよ。" }) } }]
    });
  };
  try {
    const result = await generateWithOpenAiCompatible(
      { ...sampleInput, memories: ["唐揚げが好き。", "昨日は公園へ行った。"] },
      {
        timeoutMs: 1000,
        openaiCompatible: {
          baseUrl: "http://127.0.0.1:1234/v1",
          model: "local-test"
        }
      }
    );

    assert.deepEqual(result.output, { speak: true, text: "覚えてるよ。" });
    assert.match(calls[0].body.messages[1].content, /覚えていること/);
    assert.match(calls[0].body.messages[1].content, /唐揚げが好き/);
    assert.match(calls[0].body.messages[1].content, /昨日は公園へ行った/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("openai-compatible evaluates mood without forcing response_format", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      usage: {
        prompt_tokens: 20,
        completion_tokens: 7
      },
      choices: [
        {
          message: {
            content: "```json\n{\"mood\":\"さみしい\",\"talkDesire\":82,\"topic\":\"静かな画面\"}\n```"
          }
        }
      ]
    });
  };
  try {
    const result = await evaluateMoodWithOpenAiCompatible(sampleMoodInput, {
      timeoutMs: 1000,
      openaiCompatible: {
        baseUrl: "http://127.0.0.1:1234/v1",
        model: "local-test",
        responseFormat: "json_schema"
      }
    });

    assert.deepEqual(result.output, {
      mood: "さみしい",
      talkDesire: 82,
      topic: "静かな画面"
    });
    assert.equal(calls[0].body.response_format, undefined);
    assert.match(calls[0].body.messages[1].content, /talkDesire/);
    assert.match(calls[0].body.messages[1].content, /さみしい/);
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

test("openai-compatible interprets request and normalizes choices", async (t) => {
  await t.test("formats request and maps choice", async () => {
    const originalFetch = globalThis.fetch;
    const calls = [];
    globalThis.fetch = async (url, init) => {
      calls.push({ url: String(url), init, body: JSON.parse(init.body) });
      return response(200, {
        choices: [{ message: { content: JSON.stringify({ choice: "いいえ", confidence: 0.82 }) } }]
      });
    };
    try {
      const result = await interpretWithOpenAiCompatible(sampleInterpretInput, {
        timeoutMs: 1000,
        openaiCompatible: {
          baseUrl: "http://127.0.0.1:1234/v1",
          apiKey: "secret",
          model: "local-test"
        }
      });

      assert.deepEqual(result.output, { choice: "いいえ", confidence: 0.82 });
      assert.equal(calls[0].url, "http://127.0.0.1:1234/v1/chat/completions");
      assert.equal(calls[0].body.model, "local-test");
      assert.equal(calls[0].body.temperature, 0.1);
      assert.equal(calls[0].body.response_format, undefined);
      assert.match(calls[0].body.messages[0].content, /dialogue\.interpret/);
      assert.match(calls[0].body.messages[1].content, /返事は肯定ですか/);
      assert.match(calls[0].body.messages[1].content, /あ〜うん/);
      assert.match(calls[0].body.messages[1].content, /不明/);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("json_schema format and retry preserve interpret output", async () => {
    const originalFetch = globalThis.fetch;
    const calls = [];
    globalThis.fetch = async (url, init) => {
      calls.push({ url: String(url), init, body: JSON.parse(init.body) });
      if (calls.length === 1) {
        return response(400, { error: "schema rejected" });
      }
      return response(200, {
        choices: [{ message: { content: "```json\n{\"choice\":\"はい\"}\n```" } }]
      });
    };
    try {
      const result = await interpretWithOpenAiCompatible(sampleInterpretInput, {
        timeoutMs: 1000,
        openaiCompatible: {
          baseUrl: "http://127.0.0.1:1234/v1",
          model: "local-test",
          responseFormat: "json_schema"
        }
      });

      assert.deepEqual(result.output, { choice: "はい" });
      assert.equal(calls.length, 2);
      assert.equal(calls[0].body.response_format.type, "json_schema");
      assert.equal(calls[0].body.response_format.json_schema.name, "dialogue_interpret_output");
      assert.equal(calls[1].body.response_format, undefined);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("out-of-choice and invalid JSON become unknown", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () =>
      response(200, { choices: [{ message: { content: "{\"choice\":\"maybe\"}" } }] });
    try {
      const result = await interpretWithOpenAiCompatible(sampleInterpretInput, {
        timeoutMs: 1000,
        openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "local-test" }
      });
      assert.deepEqual(result.output, { choice: "不明" });
    } finally {
      globalThis.fetch = originalFetch;
    }

    globalThis.fetch = async () =>
      response(200, { choices: [{ message: { content: "not json" } }] });
    try {
      const result = await interpretWithOpenAiCompatible(sampleInterpretInput, {
        timeoutMs: 1000,
        openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "local-test" }
      });
      assert.deepEqual(result.output, { choice: "不明" });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("API error becomes unknown", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () => response(500, { error: "nope" });
    try {
      const result = await interpretWithOpenAiCompatible(sampleInterpretInput, {
        timeoutMs: 1000,
        openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "local-test" }
      });
      assert.deepEqual(result.output, { choice: "不明" });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("timeout becomes unknown", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (_url, init) => {
      await once(init.signal, "abort");
      throw new Error("aborted");
    };
    try {
      const result = await interpretWithOpenAiCompatible(sampleInterpretInput, {
        timeoutMs: 10,
        openaiCompatible: { baseUrl: "http://127.0.0.1:1234/v1", model: "local-test" }
      });
      assert.deepEqual(result.output, { choice: "不明" });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});

test("gemini formats request and maps JSON response", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      usageMetadata: {
        promptTokenCount: 18,
        candidatesTokenCount: 7
      },
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
    assert.deepEqual(result.metadata.usage, {
      inputTokens: 18,
      outputTokens: 7,
      model: "gemini-test",
      provider: "gemini"
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("gemini evaluates mood and maps JSON response", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      usageMetadata: {
        promptTokenCount: 9,
        candidatesTokenCount: 4
      },
      candidates: [
        {
          content: {
            parts: [{ text: JSON.stringify({ mood: "ねむい", talkDesire: 12, topic: "" }) }]
          }
        }
      ]
    });
  };
  try {
    const result = await evaluateMoodWithGemini(sampleMoodInput, {
      timeoutMs: 1000,
      gemini: { apiKey: "gem-key", model: "gemini-test" }
    });

    assert.deepEqual(result.output, { mood: "ねむい", talkDesire: 12, topic: "" });
    assert.equal(calls[0].body.generationConfig.responseMimeType, "application/json");
    assert.deepEqual(calls[0].body.generationConfig.responseSchema.required, [
      "mood",
      "talkDesire",
      "topic"
    ]);
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

test("openai-compatible extracts free-form value", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      choices: [{ message: { content: JSON.stringify({ found: true, value: "ミナ" }) } }]
    });
  };
  try {
    const result = await extractWithOpenAiCompatible(sampleExtractInput, {
      timeoutMs: 1000,
      openaiCompatible: {
        baseUrl: "http://127.0.0.1:1234/v1",
        model: "local-test"
      }
    });

    assert.deepEqual(result.output, { found: true, value: "ミナ" });
    assert.equal(calls[0].body.response_format, undefined);
    assert.match(calls[0].body.messages[0].content, /dialogue\.extract/);
    assert.match(calls[0].body.messages[1].content, /ユーザーの呼び名/);
    assert.match(calls[0].body.messages[1].content, /ミナって呼んで/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("gemini interprets request and normalizes choices", async (t) => {
  await t.test("formats request and maps choice", async () => {
    const originalFetch = globalThis.fetch;
    const calls = [];
    globalThis.fetch = async (url, init) => {
      calls.push({ url: String(url), init, body: JSON.parse(init.body) });
      return response(200, {
        candidates: [{ content: { parts: [{ text: JSON.stringify({ choice: "いいえ" }) }] } }]
      });
    };
    try {
      const result = await interpretWithGemini(sampleInterpretInput, {
        timeoutMs: 1000,
        gemini: { apiKey: "gem-key", model: "gemini-test" }
      });

      assert.deepEqual(result.output, { choice: "いいえ" });
      assert.equal(
        calls[0].url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-test:generateContent?key=gem-key"
      );
      assert.equal(calls[0].body.generationConfig.temperature, 0.1);
      assert.equal(calls[0].body.generationConfig.responseMimeType, "application/json");
      assert.equal(calls[0].body.generationConfig.responseSchema.properties.choice.type, "STRING");
      assert.match(calls[0].body.contents[0].parts[0].text, /返事は肯定ですか/);
      assert.match(calls[0].body.contents[0].parts[0].text, /不明/);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  await t.test("out-of-choice, API error, and invalid JSON become unknown", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () =>
      response(200, { candidates: [{ content: { parts: [{ text: "{\"choice\":\"maybe\"}" }] } }] });
    try {
      const result = await interpretWithGemini(sampleInterpretInput, {
        timeoutMs: 1000,
        gemini: { apiKey: "gem-key", model: "gemini-test" }
      });
      assert.deepEqual(result.output, { choice: "不明" });
    } finally {
      globalThis.fetch = originalFetch;
    }

    globalThis.fetch = async () => response(500, { error: "nope" });
    try {
      const result = await interpretWithGemini(sampleInterpretInput, {
        timeoutMs: 1000,
        gemini: { apiKey: "gem-key", model: "gemini-test" }
      });
      assert.deepEqual(result.output, { choice: "不明" });
    } finally {
      globalThis.fetch = originalFetch;
    }

    globalThis.fetch = async () =>
      response(200, { candidates: [{ content: { parts: [{ text: "not json" }] } }] });
    try {
      const result = await interpretWithGemini(sampleInterpretInput, {
        timeoutMs: 1000,
        gemini: { apiKey: "gem-key", model: "gemini-test" }
      });
      assert.deepEqual(result.output, { choice: "不明" });
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});

test("gemini extracts free-form value", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init, body: JSON.parse(init.body) });
    return response(200, {
      candidates: [{ content: { parts: [{ text: JSON.stringify({ found: true, value: "ミナ" }) }] } }]
    });
  };
  try {
    const result = await extractWithGemini(sampleExtractInput, {
      timeoutMs: 1000,
      gemini: { apiKey: "gem-key", model: "gemini-test" }
    });

    assert.deepEqual(result.output, { found: true, value: "ミナ" });
    assert.equal(calls[0].body.generationConfig.responseSchema.properties.found.type, "BOOLEAN");
    assert.match(calls[0].body.contents[0].parts[0].text, /ユーザーの呼び名/);
  } finally {
    globalThis.fetch = originalFetch;
  }
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

test("capability result normalizes dialogue.interpret output", () => {
  const result = capabilityResult(
    { id: "cap_2", capability: "dialogue.interpret", input: sampleInterpretInput },
    { choice: "maybe" },
    { provider: "test" }
  );
  assert.equal(result.invocationId, "cap_2");
  assert.equal(result.capability, "dialogue.interpret");
  assert.deepEqual(result.output, { choice: "不明" });
});

test("capability result normalizes dialogue.extract output", () => {
  const ok = capabilityResult(
    { id: "cap_extract_1", capability: "dialogue.extract", input: sampleExtractInput },
    { found: true, value: " ミナ " },
    { provider: "test" }
  );
  assert.deepEqual(ok.output, { found: true, value: "ミナ" });

  for (const output of [
    { found: false, value: "ミナ" },
    { found: true, value: "" },
    { found: true, value: "x".repeat(101) }
  ]) {
    const result = capabilityResult(
      { id: "cap_extract_2", capability: "dialogue.extract", input: sampleExtractInput },
      output,
      { provider: "test" }
    );
    assert.deepEqual(result.output, { found: false, value: "不明" });
  }
});

test("capability result normalizes memory admin output", () => {
  const listed = capabilityResult(
    { id: "cap_memory_1", capability: "memory.list", input: {} },
    {
      facts: [
        {
          id: "fact-1",
          text: "  唐揚げが好き。 ",
          createdAt: "2026-01-01T00:00:00.000Z",
          updatedAt: "2026-01-02T00:00:00.000Z"
        },
        { id: "", text: "消える。" }
      ],
      episodes: [
        {
          id: "episode-1",
          text: " 公園へ行った。 ",
          timestamp: "2026-01-03T00:00:00.000Z"
        },
        { id: "episode-2", text: "" }
      ],
      episodeTotal: 4.8
    },
    {}
  );
  assert.deepEqual(listed.output, {
    facts: [
      {
        id: "fact-1",
        text: "唐揚げが好き。",
        createdAt: "2026-01-01T00:00:00.000Z",
        updatedAt: "2026-01-02T00:00:00.000Z"
      }
    ],
    episodes: [
      {
        id: "episode-1",
        text: "公園へ行った。",
        timestamp: "2026-01-03T00:00:00.000Z"
      }
    ],
    episodeTotal: 4
  });

  const updated = capabilityResult(
    { id: "cap_memory_2", capability: "memory.update", input: {} },
    { updated: true },
    {}
  );
  assert.deepEqual(updated.output, { updated: true });

  const forgotten = capabilityResult(
    { id: "cap_memory_3", capability: "memory.forget", input: {} },
    { removedFacts: 1.8, removedEpisodes: -2 },
    {}
  );
  assert.deepEqual(forgotten.output, { removedFacts: 1, removedEpisodes: 0 });
});

test("capability result normalizes mood.evaluate output", () => {
  const result = capabilityResult(
    { id: "cap_3", capability: "mood.evaluate", input: sampleMoodInput },
    { mood: "謎", talkDesire: 120.8, topic: "  机の上  " },
    { provider: "test" }
  );
  assert.equal(result.invocationId, "cap_3");
  assert.equal(result.capability, "mood.evaluate");
  assert.deepEqual(result.output, { mood: "ふつう", talkDesire: 100, topic: "机の上" });
});

test("mood output parsing normalizes vocabulary and clamps desire", () => {
  assert.deepEqual(
    parseJsonMoodEvaluateOutput("前置き {\"mood\":\"うれしい\",\"talkDesire\":88.9,\"topic\":\"散歩\"} 後置き"),
    { mood: "うれしい", talkDesire: 88, topic: "散歩" }
  );
  assert.deepEqual(
    normalizeMoodEvaluateOutput({ mood: "読めない", talkDesire: -4, topic: 12 }),
    { mood: "ふつう", talkDesire: 0, topic: "" }
  );
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

test("parseJsonInterpretOutput accepts fenced JSON but rejects non-choices", () => {
  assert.deepEqual(parseJsonInterpretOutput("```json\n{\"choice\":\"はい\"}\n```", ["はい"]), {
    choice: "はい"
  });
  assert.deepEqual(parseJsonInterpretOutput("前置き {\"choice\":\"maybe\"} 後置き", ["はい"]), {
    choice: "不明"
  });
  assert.deepEqual(parseJsonInterpretOutput("分類できません", ["はい"]), { choice: "不明" });
});

function response(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body
  };
}
