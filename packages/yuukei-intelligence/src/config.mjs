import { readFile } from "node:fs/promises";
import { join } from "node:path";

const DEFAULT_CONFIG = {
  provider: "openai-compatible",
  timeoutMs: 30_000,
  gemini: {
    model: "gemini-2.5-flash"
  },
  openaiCompatible: {
    baseUrl: "http://127.0.0.1:1234/v1",
    model: "local-model",
    responseFormat: "none"
  }
};

export async function loadConfig(cwd = process.cwd(), env = process.env) {
  const manifestConfig = await readManifestConfig(cwd);
  const merged = mergeConfig(DEFAULT_CONFIG, manifestConfig);
  if (env.YUUKEI_INTELLIGENCE_PROVIDER) {
    merged.provider = env.YUUKEI_INTELLIGENCE_PROVIDER;
  }
  if (env.YUUKEI_INTELLIGENCE_TIMEOUT_MS) {
    merged.timeoutMs = numberOrDefault(env.YUUKEI_INTELLIGENCE_TIMEOUT_MS, merged.timeoutMs);
  }
  if (env.GEMINI_API_KEY) {
    merged.gemini.apiKey = env.GEMINI_API_KEY;
  }
  if (env.GEMINI_MODEL) {
    merged.gemini.model = env.GEMINI_MODEL;
  }
  if (env.OPENAI_COMPATIBLE_BASE_URL) {
    merged.openaiCompatible.baseUrl = env.OPENAI_COMPATIBLE_BASE_URL;
  }
  if (env.OPENAI_COMPATIBLE_API_KEY) {
    merged.openaiCompatible.apiKey = env.OPENAI_COMPATIBLE_API_KEY;
  }
  if (env.OPENAI_COMPATIBLE_MODEL) {
    merged.openaiCompatible.model = env.OPENAI_COMPATIBLE_MODEL;
  }
  if (env.OPENAI_COMPATIBLE_RESPONSE_FORMAT) {
    merged.openaiCompatible.responseFormat = env.OPENAI_COMPATIBLE_RESPONSE_FORMAT;
  }
  applyHostSettings(merged, env.YUUKEI_EXTENSION_SETTINGS_JSON);
  return merged;
}

async function readManifestConfig(cwd) {
  try {
    const raw = await readFile(join(cwd, "manifest.json"), "utf8");
    const manifest = JSON.parse(raw);
    return manifest.config && typeof manifest.config === "object" ? manifest.config : {};
  } catch (error) {
    console.error(`yuukei-intelligence: manifest config unavailable: ${error.message}`);
    return {};
  }
}

function mergeConfig(base, override) {
  return {
    ...base,
    ...override,
    gemini: {
      ...base.gemini,
      ...(override?.gemini ?? {})
    },
    openaiCompatible: {
      ...base.openaiCompatible,
      ...(override?.openaiCompatible ?? {})
    }
  };
}

function numberOrDefault(value, fallback) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function applyHostSettings(config, rawSettings) {
  if (!rawSettings) {
    return;
  }
  let settings;
  try {
    settings = JSON.parse(rawSettings);
  } catch (error) {
    console.error(`yuukei-intelligence: settings json unavailable: ${error.message}`);
    return;
  }
  if (!settings || typeof settings !== "object" || Array.isArray(settings)) {
    return;
  }
  assignString(settings, "provider", (value) => {
    config.provider = value;
  });
  if (Object.hasOwn(settings, "timeoutMs")) {
    config.timeoutMs = numberOrDefault(settings.timeoutMs, config.timeoutMs);
  }
  assignString(settings, "gemini.apiKey", (value) => {
    config.gemini.apiKey = value;
  });
  assignString(settings, "gemini.model", (value) => {
    config.gemini.model = value;
  });
  assignString(settings, "openaiCompatible.baseUrl", (value) => {
    config.openaiCompatible.baseUrl = value;
  });
  assignString(settings, "openaiCompatible.apiKey", (value) => {
    config.openaiCompatible.apiKey = value;
  });
  assignString(settings, "openaiCompatible.model", (value) => {
    config.openaiCompatible.model = value;
  });
  assignString(settings, "openaiCompatible.responseFormat", (value) => {
    config.openaiCompatible.responseFormat = value;
  });
}

function assignString(settings, key, assign) {
  if (typeof settings[key] === "string") {
    assign(settings[key]);
  }
}
