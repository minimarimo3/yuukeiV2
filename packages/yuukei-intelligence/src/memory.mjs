import { randomUUID } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { summarizeMemoryIndexWithProvider } from "./providers/index.mjs";

const MAX_FACTS = 50;
const EPISODE_HALF_LIFE_DAYS = 14;

export async function indexMemory(input, config, env = process.env) {
  const events = Array.isArray(input?.events) ? input.events : [];
  if (events.length === 0) {
    return { output: { indexed: true, noteCount: 0 }, metadata: { skipped: "empty-events" } };
  }

  const summary = await summarizeMemoryIndexWithProvider(input, config);
  if (!summary.output?.diary && !Array.isArray(summary.output?.newFacts)) {
    return { output: { indexed: false }, metadata: summary.metadata ?? {} };
  }

  try {
    const store = await openMemoryStore(input, env);
    await upsertEpisode(store, {
      date: input.date,
      text: String(summary.output.diary ?? "").trim()
    });
    const noteCount = await mergeFacts(store, summary.output.newFacts ?? []);
    return {
      output: { indexed: true, noteCount },
      metadata: summary.metadata ?? {}
    };
  } catch (error) {
    console.error(`yuukei-intelligence: memory index storage failed: ${error.message}`);
    return { output: { indexed: false }, metadata: { reason: "storage-error" } };
  }
}

export async function retrieveMemory(input, env = process.env) {
  try {
    const store = await openMemoryStore(input, env);
    const query = typeof input?.query?.text === "string" ? input.query.text : "";
    const factsLimit = positiveInteger(input?.limits?.facts, 10);
    const episodesLimit = positiveInteger(input?.limits?.episodes, 5);
    const [facts, episodes] = await Promise.all([readFacts(store), readEpisodes(store)]);
    return {
      output: {
        memories: [
          ...rankFacts(facts, query).slice(0, factsLimit),
          ...rankEpisodes(episodes, query).slice(0, episodesLimit)
        ]
      },
      metadata: { facts: facts.length, episodes: episodes.length }
    };
  } catch (error) {
    console.error(`yuukei-intelligence: memory retrieve failed: ${error.message}`);
    return { output: { memories: [] }, metadata: { reason: "storage-error" } };
  }
}

export async function listMemory(input, env = process.env) {
  try {
    const store = await openMemoryStore(input, env);
    const [factsResult, episodesResult] = await Promise.all([
      readFactsWithMigration(store),
      readEpisodesWithMigration(store)
    ]);
    if (factsResult.changed) {
      await writeFacts(store, factsResult.entries);
    }
    if (episodesResult.changed) {
      await writeEpisodes(store, episodesResult.entries);
    }
    const episodeOffset = nonNegativeInteger(input?.episodeOffset, 0);
    const episodeLimit = positiveInteger(input?.episodeLimit, 50);
    const episodes = [...episodesResult.entries]
      .sort((left, right) => String(right.timestamp).localeCompare(String(left.timestamp)))
      .slice(episodeOffset, episodeOffset + episodeLimit)
      .map((episode) => ({
        id: episode.id,
        text: episode.text,
        timestamp: episode.timestamp
      }));
    return {
      output: {
        facts: factsResult.entries.map((fact) => ({
          id: fact.id,
          text: fact.text,
          createdAt: fact.createdAt,
          updatedAt: fact.updatedAt
        })),
        episodes,
        episodeTotal: episodesResult.entries.length
      },
      metadata: { facts: factsResult.entries.length, episodes: episodesResult.entries.length }
    };
  } catch (error) {
    console.error(`yuukei-intelligence: memory list failed: ${error.message}`);
    return { output: { facts: [], episodes: [], episodeTotal: 0 }, metadata: { reason: "storage-error" } };
  }
}

export async function updateMemory(input, env = process.env) {
  try {
    const text = typeof input?.text === "string" ? input.text.trim() : "";
    if (input?.kind !== "fact" || !text || text.length > 500 || typeof input?.id !== "string") {
      return { output: { updated: false }, metadata: { reason: "invalid-input" } };
    }
    const store = await openMemoryStore(input, env);
    const factsResult = await readFactsWithMigration(store);
    const facts = factsResult.entries;
    const fact = facts.find((candidate) => candidate.id === input.id);
    if (!fact) {
      if (factsResult.changed) {
        await writeFacts(store, facts);
      }
      return { output: { updated: false }, metadata: { reason: "not-found" } };
    }
    fact.text = text;
    fact.updatedAt = new Date().toISOString();
    await writeFacts(store, facts);
    return { output: { updated: true }, metadata: {} };
  } catch (error) {
    console.error(`yuukei-intelligence: memory update failed: ${error.message}`);
    return { output: { updated: false }, metadata: { reason: "storage-error" } };
  }
}

export async function forgetMemory(input, env = process.env) {
  try {
    const store = await openMemoryStore(input, env);
    const [factsResult, episodesResult] = await Promise.all([
      readFactsWithMigration(store),
      readEpisodesWithMigration(store)
    ]);
    let facts = factsResult.entries;
    let episodes = episodesResult.entries;
    const previousFacts = facts.length;
    const previousEpisodes = episodes.length;
    if (input?.all === true) {
      facts = [];
      episodes = [];
    } else {
      const entries = Array.isArray(input?.entries) ? input.entries : [];
      const factIds = new Set(
        entries
          .filter((entry) => entry?.kind === "fact" && typeof entry.id === "string")
          .map((entry) => entry.id)
      );
      const episodeIds = new Set(
        entries
          .filter((entry) => entry?.kind === "episode" && typeof entry.id === "string")
          .map((entry) => entry.id)
      );
      facts = facts.filter((fact) => !factIds.has(fact.id));
      episodes = episodes.filter((episode) => !episodeIds.has(episode.id));
    }
    await Promise.all([writeFacts(store, facts), writeEpisodes(store, episodes)]);
    return {
      output: {
        removedFacts: previousFacts - facts.length,
        removedEpisodes: previousEpisodes - episodes.length
      },
      metadata: {}
    };
  } catch (error) {
    console.error(`yuukei-intelligence: memory forget failed: ${error.message}`);
    return { output: { removedFacts: 0, removedEpisodes: 0 }, metadata: { reason: "storage-error" } };
  }
}

async function openMemoryStore(input, env) {
  const dataDir = env.YUUKEI_EXTENSION_DATA_DIR;
  if (!dataDir) {
    throw new Error("YUUKEI_EXTENSION_DATA_DIR is not configured");
  }
  const worldPackId = safeSegment(input?.worldPackId);
  const residentId = safeSegment(input?.residentId);
  const root = join(dataDir, "memory", worldPackId, residentId);
  await mkdir(root, { recursive: true });
  return {
    root,
    factsPath: join(root, "facts.json"),
    episodesPath: join(root, "episodes.jsonl")
  };
}

async function upsertEpisode(store, episode) {
  if (!episode.date || !episode.text) {
    return;
  }
  const episodes = await readEpisodes(store);
  const existing = episodes.find((candidate) => candidate.date === episode.date);
  const next = [
    ...episodes.filter((existing) => existing.date !== episode.date),
    {
      id: existing?.id ?? randomUUID(),
      date: episode.date,
      timestamp: episode.date,
      text: episode.text
    }
  ].sort((left, right) => left.date.localeCompare(right.date));
  await writeEpisodes(store, next);
}

async function mergeFacts(store, newFacts) {
  const now = new Date().toISOString();
  const facts = await readFacts(store);
  for (const fact of newFacts.map((value) => String(value).trim()).filter(Boolean)) {
    const normalized = normalizeForDuplicate(fact);
    if (!normalized) {
      continue;
    }
    const duplicate = facts.find((existing) => {
      const existingNormalized = normalizeForDuplicate(existing.text);
      return (
        existingNormalized === normalized ||
        existingNormalized.includes(normalized) ||
        normalized.includes(existingNormalized)
      );
    });
    if (duplicate) {
      duplicate.updatedAt = now;
      continue;
    }
    facts.push({ id: randomUUID(), text: fact, createdAt: now, updatedAt: now });
  }
  facts.sort((left, right) => String(right.updatedAt).localeCompare(String(left.updatedAt)));
  const trimmed = facts.slice(0, MAX_FACTS);
  await writeFacts(store, trimmed);
  return trimmed.length;
}

async function readFacts(store) {
  return (await readFactsWithMigration(store)).entries;
}

async function readFactsWithMigration(store) {
  try {
    const value = JSON.parse(await readFile(store.factsPath, "utf8"));
    if (!Array.isArray(value)) {
      return { entries: [], changed: false };
    }
    let changed = false;
    const entries = value
      .filter((fact) => fact && typeof fact === "object" && typeof fact.text === "string")
      .map((fact) => {
        const id = typeof fact.id === "string" && fact.id.trim() ? fact.id : randomUUID();
        if (id !== fact.id) {
          changed = true;
        }
        return {
          id,
          text: fact.text,
          createdAt: typeof fact.createdAt === "string" ? fact.createdAt : "",
          updatedAt: typeof fact.updatedAt === "string" ? fact.updatedAt : ""
        };
      });
    return { entries, changed };
  } catch (error) {
    if (error.code === "ENOENT") {
      return { entries: [], changed: false };
    }
    throw error;
  }
}

async function writeFacts(store, facts) {
  await writeFile(store.factsPath, `${JSON.stringify(facts, null, 2)}\n`);
}

async function readEpisodes(store) {
  return (await readEpisodesWithMigration(store)).entries;
}

async function readEpisodesWithMigration(store) {
  try {
    const raw = await readFile(store.episodesPath, "utf8");
    let changed = false;
    const entries = raw
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => JSON.parse(line))
      .filter(
        (episode) =>
          episode &&
          typeof episode === "object" &&
          (typeof episode.date === "string" || typeof episode.timestamp === "string") &&
          typeof episode.text === "string"
      )
      .map((episode) => {
        const id = typeof episode.id === "string" && episode.id.trim() ? episode.id : randomUUID();
        const timestamp =
          typeof episode.timestamp === "string" && episode.timestamp.trim()
            ? episode.timestamp
            : episode.date;
        if (id !== episode.id || timestamp !== episode.timestamp) {
          changed = true;
        }
        return { id, date: timestamp.slice(0, 10), timestamp, text: episode.text };
      });
    return { entries, changed };
  } catch (error) {
    if (error.code === "ENOENT") {
      return { entries: [], changed: false };
    }
    throw error;
  }
}

async function writeEpisodes(store, episodes) {
  const lines = episodes.map((episode) => JSON.stringify(episode)).join("\n");
  await writeFile(store.episodesPath, lines ? `${lines}\n` : "");
}

function rankFacts(facts, query) {
  return facts
    .map((fact) => ({
      text: fact.text,
      kind: "fact",
      score: bigramScore(query, fact.text),
      updatedAt: fact.updatedAt
    }))
    .sort((left, right) => right.score - left.score || right.updatedAt.localeCompare(left.updatedAt))
    .map(({ text, kind }) => ({ text, kind }));
}

function rankEpisodes(episodes, query) {
  const today = new Date();
  return episodes
    .map((episode) => {
      const relevance = bigramScore(query, episode.text);
      const score =
        relevance > 0
          ? relevance + 0.1 * recencyDecay(today, parseDate(episode.date))
          : 0;
      return { text: episode.text, kind: "episode", date: episode.date, score };
    })
    .filter((episode) => episode.score > 0)
    .sort((left, right) => right.score - left.score || right.date.localeCompare(left.date))
    .map(({ text, kind, date }) => ({ text, kind, date }));
}

function bigramScore(query, text) {
  const queryBigrams = bigrams(query);
  if (queryBigrams.size === 0) {
    return 0;
  }
  const textBigrams = bigrams(text);
  let overlap = 0;
  for (const bigram of queryBigrams) {
    if (textBigrams.has(bigram)) {
      overlap += 1;
    }
  }
  return overlap / queryBigrams.size;
}

function bigrams(value) {
  const chars = [...normalizeForSearch(value)];
  if (chars.length === 0) {
    return new Set();
  }
  if (chars.length === 1) {
    return new Set(chars);
  }
  const output = new Set();
  for (let index = 0; index < chars.length - 1; index += 1) {
    output.add(`${chars[index]}${chars[index + 1]}`);
  }
  return output;
}

function recencyDecay(today, date) {
  if (!date) {
    return 0;
  }
  const ageMs = Math.max(0, today.getTime() - date.getTime());
  const ageDays = ageMs / 86_400_000;
  return Math.pow(0.5, ageDays / EPISODE_HALF_LIFE_DAYS);
}

function parseDate(value) {
  const date = new Date(`${value}T00:00:00.000Z`);
  return Number.isNaN(date.getTime()) ? null : date;
}

function normalizeForSearch(value) {
  return String(value ?? "")
    .toLowerCase()
    .normalize("NFKC")
    .replace(/\s+/g, "");
}

function normalizeForDuplicate(value) {
  return normalizeForSearch(value).replace(/[。、，,.!?！？]/g, "");
}

function positiveInteger(value, fallback) {
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? Math.trunc(number) : fallback;
}

function nonNegativeInteger(value, fallback) {
  const number = Number(value);
  return Number.isFinite(number) && number >= 0 ? Math.trunc(number) : fallback;
}

function safeSegment(value) {
  const segment = String(value ?? "").trim();
  return segment ? segment.replace(/[^a-zA-Z0-9._-]/g, "_") : "default";
}
