import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { loadConfig } from "../src/config.mjs";

test("host settings json has priority over manifest config and env", async () => {
  const dir = await mkdtemp(join(tmpdir(), "yuukei-intelligence-config-"));
  await writeFile(
    join(dir, "manifest.json"),
    JSON.stringify({
      config: {
        provider: "gemini",
        timeoutMs: 120000,
        gemini: { model: "manifest-gemini" },
        openaiCompatible: {
          baseUrl: "http://manifest.local/v1",
          model: "manifest-model",
          responseFormat: "json_schema"
        }
      }
    })
  );

  try {
    const config = await loadConfig(dir, {
      YUUKEI_INTELLIGENCE_PROVIDER: "gemini",
      YUUKEI_INTELLIGENCE_TIMEOUT_MS: "90000",
      GEMINI_API_KEY: "env-gemini-key",
      GEMINI_MODEL: "env-gemini",
      OPENAI_COMPATIBLE_BASE_URL: "http://env.local/v1",
      OPENAI_COMPATIBLE_API_KEY: "env-openai-key",
      OPENAI_COMPATIBLE_MODEL: "env-model",
      OPENAI_COMPATIBLE_RESPONSE_FORMAT: "json_object",
      YUUKEI_EXTENSION_SETTINGS_JSON: JSON.stringify({
        provider: "openai-compatible",
        timeoutMs: 30000,
        "gemini.apiKey": "settings-gemini-key",
        "gemini.model": "settings-gemini",
        "openaiCompatible.baseUrl": "http://settings.local/v1",
        "openaiCompatible.apiKey": "settings-openai-key",
        "openaiCompatible.model": "settings-model",
        "openaiCompatible.responseFormat": "none"
      })
    });

    assert.equal(config.provider, "openai-compatible");
    assert.equal(config.timeoutMs, 30000);
    assert.equal(config.gemini.apiKey, "settings-gemini-key");
    assert.equal(config.gemini.model, "settings-gemini");
    assert.equal(config.openaiCompatible.baseUrl, "http://settings.local/v1");
    assert.equal(config.openaiCompatible.apiKey, "settings-openai-key");
    assert.equal(config.openaiCompatible.model, "settings-model");
    assert.equal(config.openaiCompatible.responseFormat, "none");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
