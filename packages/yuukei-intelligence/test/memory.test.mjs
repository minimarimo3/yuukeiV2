import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { indexMemory, retrieveMemory } from "../src/memory.mjs";

const config = {
  provider: "openai-compatible",
  timeoutMs: 1000,
  openaiCompatible: {
    baseUrl: "http://stub.local/v1",
    model: "stub-model"
  }
};

test("memory.index saves diary and facts", async () => {
  const dataDir = await tempDataDir();
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (_url, init) => {
    const body = JSON.parse(init.body);
    assert.match(body.messages[1].content, /唐揚げを食べた/);
    return chatResponse({ diary: "ユーザーは唐揚げの話をした。", newFacts: ["唐揚げが好き。"] });
  };
  try {
    const result = await indexMemory(sampleIndexInput(), config, env(dataDir));
    assert.deepEqual(result.output, { indexed: true, noteCount: 1 });
    assert.deepEqual(await readEpisodes(dataDir), [
      { date: "2026-01-02", text: "ユーザーは唐揚げの話をした。" }
    ]);
    assert.equal((await readFacts(dataDir))[0].text, "唐揚げが好き。");
  } finally {
    globalThis.fetch = originalFetch;
    await rm(dataDir, { recursive: true, force: true });
  }
});

test("memory.index replaces same-date diary", async () => {
  const dataDir = await tempDataDir();
  const originalFetch = globalThis.fetch;
  let count = 0;
  globalThis.fetch = async () => {
    count += 1;
    return chatResponse({
      diary: count === 1 ? "古い日記。" : "新しい日記。",
      newFacts: []
    });
  };
  try {
    await indexMemory(sampleIndexInput(), config, env(dataDir));
    await indexMemory(sampleIndexInput(), config, env(dataDir));
    assert.deepEqual(await readEpisodes(dataDir), [{ date: "2026-01-02", text: "新しい日記。" }]);
  } finally {
    globalThis.fetch = originalFetch;
    await rm(dataDir, { recursive: true, force: true });
  }
});

test("memory.index avoids duplicate facts and keeps max 50", async () => {
  const dataDir = await tempDataDir();
  const originalFetch = globalThis.fetch;
  let count = 0;
  globalThis.fetch = async () => {
    count += 1;
    const facts =
      count === 1
        ? ["唐揚げが好き。", "唐揚げ が 好き"]
        : Array.from({ length: 5 }, (_, index) => `恒久ノート ${count}-${index}`);
    return chatResponse({ diary: `日記${count}`, newFacts: facts });
  };
  try {
    await indexMemory(sampleIndexInput(), config, env(dataDir));
    assert.equal((await readFacts(dataDir)).filter((fact) => fact.text.includes("唐揚げ")).length, 1);
    for (let index = 1; index < 12; index += 1) {
      await indexMemory({ ...sampleIndexInput(), date: `2026-01-${String(index + 1).padStart(2, "0")}` }, config, env(dataDir));
    }
    const facts = await readFacts(dataDir);
    assert.equal(facts.length, 50);
  } finally {
    globalThis.fetch = originalFetch;
    await rm(dataDir, { recursive: true, force: true });
  }
});

test("memory.index returns indexed false on API error", async () => {
  const dataDir = await tempDataDir();
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response(JSON.stringify({ error: "nope" }), { status: 500 });
  try {
    const result = await indexMemory(sampleIndexInput(), config, env(dataDir));
    assert.deepEqual(result.output, { indexed: false });
  } finally {
    globalThis.fetch = originalFetch;
    await rm(dataDir, { recursive: true, force: true });
  }
});

test("memory.index skips empty event days without LLM", async () => {
  const dataDir = await tempDataDir();
  const originalFetch = globalThis.fetch;
  let called = false;
  globalThis.fetch = async () => {
    called = true;
    return chatResponse({ diary: "呼ばれない", newFacts: [] });
  };
  try {
    const result = await indexMemory({ ...sampleIndexInput(), events: [] }, config, env(dataDir));
    assert.deepEqual(result.output, { indexed: true, noteCount: 0 });
    assert.equal(called, false);
  } finally {
    globalThis.fetch = originalFetch;
    await rm(dataDir, { recursive: true, force: true });
  }
});

test("memory.retrieve ranks matching episodes and fills facts to limit", async () => {
  const dataDir = await tempDataDir();
  await seedMemory(dataDir, {
    facts: [
      { text: "朝はコーヒーを飲む。", createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" },
      { text: "猫の写真が好き。", createdAt: "2026-01-02T00:00:00.000Z", updatedAt: "2026-01-02T00:00:00.000Z" },
      { text: "夜に散歩する。", createdAt: "2026-01-03T00:00:00.000Z", updatedAt: "2026-01-03T00:00:00.000Z" }
    ],
    episodes: [
      { date: "2026-01-01", text: "昔、映画の話をした。" },
      { date: todayString(), text: "今日、唐揚げ定食を楽しみにしていた。" }
    ]
  });

  const result = await retrieveMemory(
    {
      residentId: "resident-default",
      worldPackId: "default-yuukei",
      query: { text: "唐揚げ" },
      limits: { facts: 2, episodes: 1 }
    },
    env(dataDir)
  );

  assert.deepEqual(result.output.memories.slice(0, 2).map((memory) => memory.kind), [
    "fact",
    "fact"
  ]);
  const episode = result.output.memories.find((memory) => memory.kind === "episode");
  assert.equal(episode.text, "今日、唐揚げ定食を楽しみにしていた。");
  assert.equal(episode.date, todayString());
  assert.equal(result.output.memories.filter((memory) => memory.kind === "fact").length, 2);
  await rm(dataDir, { recursive: true, force: true });
});

test("memory storage is separated by worldPackId and residentId", async () => {
  const dataDir = await tempDataDir();
  await seedMemory(dataDir, {
    worldPackId: "world-a",
    residentId: "resident-a",
    facts: [{ text: "Aだけの事実。", createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" }],
    episodes: [{ date: "2026-01-01", text: "Aだけの日記。" }]
  });

  const result = await retrieveMemory(
    {
      residentId: "resident-b",
      worldPackId: "world-a",
      query: { text: "事実" },
      limits: { facts: 10, episodes: 5 }
    },
    env(dataDir)
  );
  assert.deepEqual(result.output, { memories: [] });
  await rm(dataDir, { recursive: true, force: true });
});

function sampleIndexInput() {
  return {
    residentId: "resident-default",
    worldPackId: "default-yuukei",
    date: "2026-01-02",
    events: [
      {
        kind: "conversation.text",
        timestamp: "2026-01-02T12:00:00.000Z",
        payload: { text: "唐揚げを食べた" }
      }
    ]
  };
}

async function tempDataDir() {
  return await mkdtemp(join(tmpdir(), "yuukei-memory-test-"));
}

function env(dataDir) {
  return { YUUKEI_EXTENSION_DATA_DIR: dataDir };
}

function chatResponse(value) {
  return new Response(
    JSON.stringify({ choices: [{ message: { content: JSON.stringify(value) } }] }),
    { status: 200, headers: { "content-type": "application/json" } }
  );
}

async function readFacts(dataDir) {
  return JSON.parse(await readFile(memoryPath(dataDir, "facts.json"), "utf8"));
}

async function readEpisodes(dataDir) {
  return (await readFile(memoryPath(dataDir, "episodes.jsonl"), "utf8"))
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

async function seedMemory(dataDir, { worldPackId = "default-yuukei", residentId = "resident-default", facts = [], episodes = [] }) {
  const root = join(dataDir, "memory", worldPackId, residentId);
  await mkdir(root, { recursive: true });
  await writeFile(join(root, "facts.json"), `${JSON.stringify(facts, null, 2)}\n`);
  await writeFile(join(root, "episodes.jsonl"), `${episodes.map((episode) => JSON.stringify(episode)).join("\n")}\n`);
}

function memoryPath(dataDir, fileName) {
  return join(dataDir, "memory", "default-yuukei", "resident-default", fileName);
}

function todayString() {
  return new Date().toISOString().slice(0, 10);
}
