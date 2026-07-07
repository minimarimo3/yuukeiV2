import assert from "node:assert/strict";
import { mkdtemp, readdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  loadConfig,
  parseSpeakerOverrides,
  resolveSpeaker,
  synthesizeWithVoicevox,
  voicevoxCapabilityResult,
  wavDurationMs
} from "../src/voicevox.mjs";

test("speakerOverrides resolves actor-specific VOICEVOX styles and ignores invalid entries", () => {
  const map = parseSpeakerOverrides("yuukei=2, partner = 3, broken, bad=x, negative=-1");
  const config = { speaker: 9, speakerOverrideMap: map };
  assert.equal(resolveSpeaker(config, "yuukei"), 2);
  assert.equal(resolveSpeaker(config, "partner"), 3);
  assert.equal(resolveSpeaker(config, "unknown"), 9);
  assert.equal(map.has("broken"), false);
  assert.equal(map.has("bad"), false);
  assert.equal(map.has("negative"), false);
});

test("loadConfig merges host settings and prepares data directory", () => {
  const config = loadConfig({
    YUUKEI_EXTENSION_SETTINGS_JSON: JSON.stringify({
      baseUrl: "http://localhost:50021/",
      speaker: 8,
      speedScale: 1.25,
      speakerOverrides: "yuukei=10"
    }),
    YUUKEI_EXTENSION_DATA_DIR: "/tmp/yuukei-voicevox-test"
  });
  assert.equal(config.baseUrl, "http://localhost:50021");
  assert.equal(config.speaker, 8);
  assert.equal(config.speedScale, 1.25);
  assert.equal(config.speakerOverrideMap.get("yuukei"), 10);
  assert.equal(config.dataDir, "/tmp/yuukei-voicevox-test");
});

test("synthesizeWithVoicevox calls audio_query then synthesis and returns wav file output", async () => {
  const dataDir = await mkdtemp(join(tmpdir(), "yuukei-voicevox-"));
  const calls = [];
  const wav = oneSecondPcmWav();
  const fetchImpl = async (url, options) => {
    calls.push({ url: String(url), options });
    if (String(url).includes("/audio_query")) {
      return {
        ok: true,
        status: 200,
        async json() {
          return { speedScale: 1, accentPhrases: [] };
        }
      };
    }
    return {
      ok: true,
      status: 200,
      async arrayBuffer() {
        return wav.buffer.slice(wav.byteOffset, wav.byteOffset + wav.byteLength);
      }
    };
  };
  const output = await synthesizeWithVoicevox(
    {
      id: "cap_1",
      capability: "speech.synthesis",
      input: {
        text: "こんにちは",
        speakerId: "partner",
        displayCommandId: "cmd_1"
      }
    },
    {
      baseUrl: "http://voicevox.local",
      speaker: 2,
      speedScale: 1.4,
      speakerOverrideMap: new Map([["partner", 3]]),
      dataDir
    },
    { fetchImpl, now: () => 12345 }
  );

  assert.equal(calls.length, 2);
  assert.match(calls[0].url, /\/audio_query\?speaker=3&text=/);
  assert.equal(new URL(calls[0].url).searchParams.get("text"), "こんにちは");
  assert.match(calls[1].url, /\/synthesis\?speaker=3/);
  assert.deepEqual(JSON.parse(calls[1].options.body), {
    speedScale: 1.4,
    accentPhrases: []
  });
  assert.match(output.audioPath, /cmd_1-12345\.wav$/);
  assert.equal(output.durationMs, 1000);
  assert.equal(output.format, "wav");
  assert.equal((await stat(output.audioPath)).isFile(), true);
  await rm(dataDir, { recursive: true, force: true });
});

test("synthesizeWithVoicevox rejects text longer than 400 characters before HTTP", async () => {
  let called = false;
  await assert.rejects(
    () =>
      synthesizeWithVoicevox(
        {
          id: "cap_long",
          capability: "speech.synthesis",
          input: { text: "あ".repeat(401), speakerId: "yuukei" }
        },
        {
          baseUrl: "http://voicevox.local",
          speaker: 2,
          speedScale: 1,
          speakerOverrideMap: new Map(),
          dataDir: join(tmpdir(), "yuukei-voicevox-long")
        },
        {
          fetchImpl: async () => {
            called = true;
          }
        }
      ),
    /400/
  );
  assert.equal(called, false);
});

test("cleanup keeps only the latest twenty wav files", async () => {
  const dataDir = await mkdtemp(join(tmpdir(), "yuukei-voicevox-cleanup-"));
  const wav = oneSecondPcmWav();
  const fetchImpl = async (url) => {
    if (String(url).includes("/audio_query")) {
      return {
        ok: true,
        status: 200,
        async json() {
          return {};
        }
      };
    }
    return {
      ok: true,
      status: 200,
      async arrayBuffer() {
        return wav.buffer.slice(wav.byteOffset, wav.byteOffset + wav.byteLength);
      }
    };
  };
  for (let index = 0; index < 22; index += 1) {
    await synthesizeWithVoicevox(
      {
        id: `cap_${index}`,
        capability: "speech.synthesis",
        input: { text: "声", displayCommandId: `cmd_${index}` }
      },
      {
        baseUrl: "http://voicevox.local",
        speaker: 2,
        speedScale: 1,
        speakerOverrideMap: new Map(),
        dataDir
      },
      { fetchImpl, now: () => index }
    );
  }
  const wavFiles = (await readdir(dataDir)).filter((name) => name.endsWith(".wav"));
  assert.equal(wavFiles.length, 20);
  await rm(dataDir, { recursive: true, force: true });
});

test("capability result normalizes speech synthesis output", () => {
  const result = voicevoxCapabilityResult(
    { id: "cap_1" },
    { audioPath: "/tmp/a.wav", durationMs: 123.6, format: "wav" }
  );
  assert.equal(result.invocationId, "cap_1");
  assert.equal(result.extensionId, "yuukei-voicevox");
  assert.equal(result.capability, "speech.synthesis");
  assert.deepEqual(result.output, {
    audioPath: "/tmp/a.wav",
    durationMs: 124,
    format: "wav"
  });
});

test("wavDurationMs returns zero for invalid wav data", () => {
  assert.equal(wavDurationMs(Buffer.from("nope")), 0);
});

function oneSecondPcmWav() {
  const sampleRate = 8000;
  const channels = 1;
  const bitsPerSample = 16;
  const byteRate = sampleRate * channels * (bitsPerSample / 8);
  const dataSize = byteRate;
  const buffer = Buffer.alloc(44 + dataSize);
  buffer.write("RIFF", 0, "ascii");
  buffer.writeUInt32LE(36 + dataSize, 4);
  buffer.write("WAVE", 8, "ascii");
  buffer.write("fmt ", 12, "ascii");
  buffer.writeUInt32LE(16, 16);
  buffer.writeUInt16LE(1, 20);
  buffer.writeUInt16LE(channels, 22);
  buffer.writeUInt32LE(sampleRate, 24);
  buffer.writeUInt32LE(byteRate, 28);
  buffer.writeUInt16LE(channels * (bitsPerSample / 8), 32);
  buffer.writeUInt16LE(bitsPerSample, 34);
  buffer.write("data", 36, "ascii");
  buffer.writeUInt32LE(dataSize, 40);
  return buffer;
}
