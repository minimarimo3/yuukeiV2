import { mkdir, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const EXTENSION_ID = "yuukei-voicevox";
const DEFAULT_CONFIG = {
  baseUrl: "http://127.0.0.1:50021",
  speaker: 2,
  speedScale: 1.0,
  speakerOverrides: "yuukei=2,partner=3"
};
const MAX_TEXT_LENGTH = 400;
const MAX_WAV_FILES = 20;

export function loadConfig(env = process.env) {
  const config = { ...DEFAULT_CONFIG };
  applyHostSettings(config, env.YUUKEI_EXTENSION_SETTINGS_JSON);
  config.baseUrl = normalizeBaseUrl(config.baseUrl);
  config.speaker = positiveIntegerOrDefault(config.speaker, DEFAULT_CONFIG.speaker);
  config.speedScale = positiveNumberOrDefault(config.speedScale, DEFAULT_CONFIG.speedScale);
  config.speakerOverrideMap = parseSpeakerOverrides(config.speakerOverrides);
  config.dataDir =
    typeof env.YUUKEI_EXTENSION_DATA_DIR === "string" && env.YUUKEI_EXTENSION_DATA_DIR.trim()
      ? env.YUUKEI_EXTENSION_DATA_DIR
      : join(tmpdir(), "yuukei-voicevox");
  return config;
}

export async function synthesizeWithVoicevox(
  invocation,
  config,
  { fetchImpl = globalThis.fetch, now = () => Date.now() } = {}
) {
  if (invocation?.capability !== "speech.synthesis") {
    throw new Error("unsupported capability");
  }
  if (typeof fetchImpl !== "function") {
    throw new Error("fetch is unavailable");
  }
  const input = invocation.input && typeof invocation.input === "object" ? invocation.input : {};
  const text = typeof input.text === "string" ? input.text : "";
  if (!text.trim()) {
    throw new Error("text is empty");
  }
  if ([...text].length > MAX_TEXT_LENGTH) {
    throw new Error("text exceeds 400 characters");
  }

  const speakerId = typeof input.speakerId === "string" ? input.speakerId : "";
  const speaker = resolveSpeaker(config, speakerId);
  await cleanupOldWavs(config.dataDir);
  const query = await requestAudioQuery(fetchImpl, config.baseUrl, speaker, text);
  query.speedScale = config.speedScale;
  const wav = await requestSynthesis(fetchImpl, config.baseUrl, speaker, query);
  const audioPath = await writeWav(config.dataDir, invocation, now(), wav);
  await cleanupOldWavs(config.dataDir);
  return {
    audioPath,
    durationMs: wavDurationMs(wav),
    format: "wav"
  };
}

export function resolveSpeaker(config, actorId) {
  const override = config.speakerOverrideMap?.get(actorId);
  return override ?? config.speaker;
}

export function parseSpeakerOverrides(value) {
  const map = new Map();
  if (typeof value !== "string") {
    return map;
  }
  for (const entry of value.split(",")) {
    const [rawActorId, rawStyleId] = entry.split("=");
    const actorId = rawActorId?.trim();
    const styleId = Number(rawStyleId);
    if (!actorId || !Number.isInteger(styleId) || styleId < 0) {
      continue;
    }
    map.set(actorId, styleId);
  }
  return map;
}

export function voicevoxCapabilityResult(invocation, output) {
  return {
    invocationId: invocation?.id ?? "",
    extensionId: EXTENSION_ID,
    capability: "speech.synthesis",
    output: normalizeOutput(output),
    metadata: {}
  };
}

export function wavDurationMs(bytes) {
  const buffer = Buffer.from(bytes);
  if (buffer.length < 44 || buffer.toString("ascii", 0, 4) !== "RIFF") {
    return 0;
  }
  let offset = 12;
  let byteRate = 0;
  let dataSize = 0;
  while (offset + 8 <= buffer.length) {
    const id = buffer.toString("ascii", offset, offset + 4);
    const size = buffer.readUInt32LE(offset + 4);
    const start = offset + 8;
    if (id === "fmt " && size >= 16 && start + 12 <= buffer.length) {
      byteRate = buffer.readUInt32LE(start + 8);
    } else if (id === "data") {
      dataSize = size;
    }
    offset = start + size + (size % 2);
  }
  if (byteRate <= 0 || dataSize <= 0) {
    return 0;
  }
  return Math.max(0, Math.round((dataSize / byteRate) * 1000));
}

async function requestAudioQuery(fetchImpl, baseUrl, speaker, text) {
  const url = new URL(`${baseUrl}/audio_query`);
  url.searchParams.set("speaker", String(speaker));
  url.searchParams.set("text", text);
  const response = await fetchImpl(url, { method: "POST" });
  if (!response.ok) {
    throw new Error(`audio_query failed: ${response.status}`);
  }
  const query = await response.json();
  if (!query || typeof query !== "object" || Array.isArray(query)) {
    throw new Error("audio_query returned invalid json");
  }
  return query;
}

async function requestSynthesis(fetchImpl, baseUrl, speaker, query) {
  const url = new URL(`${baseUrl}/synthesis`);
  url.searchParams.set("speaker", String(speaker));
  const response = await fetchImpl(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(query)
  });
  if (!response.ok) {
    throw new Error(`synthesis failed: ${response.status}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

async function writeWav(dataDir, invocation, timestamp, wav) {
  await mkdir(dataDir, { recursive: true });
  const commandId =
    typeof invocation?.input?.displayCommandId === "string" && invocation.input.displayCommandId
      ? invocation.input.displayCommandId
      : invocation?.id ?? "speech";
  const filename = `${sanitizePathSegment(commandId)}-${timestamp}.wav`;
  const audioPath = join(dataDir, filename);
  await writeFile(audioPath, wav);
  return audioPath;
}

async function cleanupOldWavs(dataDir) {
  await mkdir(dataDir, { recursive: true });
  const entries = await readdir(dataDir);
  const wavs = [];
  for (const name of entries) {
    if (!name.endsWith(".wav")) {
      continue;
    }
    const path = join(dataDir, name);
    try {
      const info = await stat(path);
      wavs.push({ path, mtimeMs: info.mtimeMs });
    } catch {
      // The file may have disappeared between readdir and stat.
    }
  }
  wavs.sort((a, b) => b.mtimeMs - a.mtimeMs);
  await Promise.all(wavs.slice(MAX_WAV_FILES).map((entry) => rm(entry.path, { force: true })));
}

function applyHostSettings(config, rawSettings) {
  if (!rawSettings) {
    return;
  }
  let settings;
  try {
    settings = JSON.parse(rawSettings);
  } catch {
    return;
  }
  if (!settings || typeof settings !== "object" || Array.isArray(settings)) {
    return;
  }
  assignString(settings, "baseUrl", (value) => {
    config.baseUrl = value;
  });
  if (Object.hasOwn(settings, "speaker")) {
    config.speaker = positiveIntegerOrDefault(settings.speaker, config.speaker);
  }
  if (Object.hasOwn(settings, "speedScale")) {
    config.speedScale = positiveNumberOrDefault(settings.speedScale, config.speedScale);
  }
  assignString(settings, "speakerOverrides", (value) => {
    config.speakerOverrides = value;
  });
}

function assignString(settings, key, assign) {
  if (typeof settings[key] === "string") {
    assign(settings[key]);
  }
}

function normalizeBaseUrl(value) {
  return String(value || DEFAULT_CONFIG.baseUrl).replace(/\/+$/, "");
}

function positiveIntegerOrDefault(value, fallback) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : fallback;
}

function positiveNumberOrDefault(value, fallback) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function sanitizePathSegment(value) {
  return String(value).replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 80) || "speech";
}

function normalizeOutput(output) {
  return {
    audioPath: typeof output?.audioPath === "string" ? output.audioPath : "",
    durationMs:
      typeof output?.durationMs === "number" && Number.isFinite(output.durationMs)
        ? Math.max(0, Math.round(output.durationMs))
        : 0,
    format: "wav"
  };
}
