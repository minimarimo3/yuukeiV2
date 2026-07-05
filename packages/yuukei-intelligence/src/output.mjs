export function silentOutput() {
  return { speak: false };
}

export function unknownChoiceOutput() {
  return { choice: "不明" };
}

export function memoryIndexFailureOutput() {
  return { indexed: false };
}

export function normalizeOutput(value, maxLength = 120) {
  if (!value || typeof value !== "object" || typeof value.speak !== "boolean") {
    return silentOutput();
  }
  if (!value.speak) {
    return silentOutput();
  }
  const text = typeof value.text === "string" ? value.text.trim() : "";
  if (!text) {
    return silentOutput();
  }
  const output = {
    speak: true,
    text: text.slice(0, Math.max(1, maxLength))
  };
  if (typeof value.expression === "string" && value.expression.trim()) {
    output.expression = value.expression.trim();
  }
  if (typeof value.motion === "string" && value.motion.trim()) {
    output.motion = value.motion.trim();
  }
  return output;
}

export function normalizeInterpretOutput(value, choices = []) {
  if (!value || typeof value !== "object" || typeof value.choice !== "string") {
    return unknownChoiceOutput();
  }
  const choice = value.choice.trim();
  const allowed = new Set([...choices, "不明"]);
  if (!allowed.has(choice)) {
    return unknownChoiceOutput();
  }
  const output = { choice };
  if (typeof value.confidence === "number" && Number.isFinite(value.confidence)) {
    output.confidence = value.confidence;
  }
  return output;
}

export function parseJsonOutput(text, maxLength) {
  if (typeof text !== "string" || !text.trim()) {
    return silentOutput();
  }
  const candidates = [stripCodeFence(text), extractJsonObject(text)].filter(Boolean);
  const uniqueCandidates = [...new Set(candidates)];
  let lastError;
  for (const candidate of uniqueCandidates) {
    try {
      return normalizeOutput(JSON.parse(candidate), maxLength);
    } catch (error) {
      lastError = error;
    }
  }
  console.error(
    `yuukei-intelligence: failed to parse provider JSON: ${lastError?.message ?? "no JSON object found"}`
  );
  return silentOutput();
}

export function parseJsonInterpretOutput(text, choices) {
  if (typeof text !== "string" || !text.trim()) {
    return unknownChoiceOutput();
  }
  const candidates = [stripCodeFence(text), extractJsonObject(text)].filter(Boolean);
  const uniqueCandidates = [...new Set(candidates)];
  let lastError;
  for (const candidate of uniqueCandidates) {
    try {
      return normalizeInterpretOutput(JSON.parse(candidate), choices);
    } catch (error) {
      lastError = error;
    }
  }
  console.error(
    `yuukei-intelligence: failed to parse provider JSON: ${lastError?.message ?? "no JSON object found"}`
  );
  return unknownChoiceOutput();
}

export function parseJsonMemoryIndexOutput(text) {
  if (typeof text !== "string" || !text.trim()) {
    return null;
  }
  const candidates = [stripCodeFence(text), extractJsonObject(text)].filter(Boolean);
  const uniqueCandidates = [...new Set(candidates)];
  let lastError;
  for (const candidate of uniqueCandidates) {
    try {
      return normalizeMemoryIndexSummary(JSON.parse(candidate));
    } catch (error) {
      lastError = error;
    }
  }
  console.error(
    `yuukei-intelligence: failed to parse memory index JSON: ${lastError?.message ?? "no JSON object found"}`
  );
  return null;
}

export function normalizeMemoryIndexSummary(value) {
  if (!value || typeof value !== "object") {
    return null;
  }
  const diary = typeof value.diary === "string" ? value.diary.trim() : "";
  const newFacts = Array.isArray(value.newFacts)
    ? value.newFacts
        .filter((fact) => typeof fact === "string")
        .map((fact) => fact.trim())
        .filter(Boolean)
        .slice(0, 5)
    : [];
  if (!diary && newFacts.length === 0) {
    return null;
  }
  return { diary, newFacts };
}

function stripCodeFence(text) {
  const trimmed = text.trim();
  const match = trimmed.match(/^```(?:json)?\s*([\s\S]*?)\s*```$/i);
  return match ? match[1].trim() : trimmed;
}

function extractJsonObject(text) {
  const source = stripCodeFence(text);
  const start = source.indexOf("{");
  if (start < 0) {
    return null;
  }
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = start; index < source.length; index += 1) {
    const char = source[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === "\"") {
        inString = false;
      }
      continue;
    }
    if (char === "\"") {
      inString = true;
    } else if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(start, index + 1);
      }
    }
  }
  return null;
}

export function capabilityResult(invocation, output, metadata = {}) {
  const capability = invocation?.capability ?? "dialogue.generate";
  let normalizedOutput;
  if (capability === "dialogue.interpret") {
    normalizedOutput = normalizeInterpretOutput(output, invocation?.input?.choices);
  } else if (capability === "memory.index") {
    normalizedOutput = normalizeMemoryIndexCapabilityOutput(output);
  } else if (capability === "memory.retrieve") {
    normalizedOutput = normalizeMemoryRetrieveCapabilityOutput(output);
  } else {
    normalizedOutput = normalizeOutput(output, invocation?.input?.constraints?.maxLength);
  }
  return {
    invocationId: invocation?.id ?? "",
    extensionId: "yuukei-intelligence",
    capability,
    output: normalizedOutput,
    metadata
  };
}

function normalizeMemoryIndexCapabilityOutput(value) {
  if (!value || typeof value !== "object") {
    return memoryIndexFailureOutput();
  }
  const output = { indexed: value.indexed === true };
  if (typeof value.noteCount === "number" && Number.isFinite(value.noteCount)) {
    output.noteCount = Math.max(0, Math.trunc(value.noteCount));
  }
  return output;
}

function normalizeMemoryRetrieveCapabilityOutput(value) {
  if (!value || typeof value !== "object" || !Array.isArray(value.memories)) {
    return { memories: [] };
  }
  return {
    memories: value.memories
      .filter((memory) => memory && typeof memory === "object")
      .map((memory) => {
        const text = typeof memory.text === "string" ? memory.text.trim() : "";
        const kind = memory.kind === "episode" ? "episode" : "fact";
        const output = { text, kind };
        if (kind === "episode" && typeof memory.date === "string" && memory.date.trim()) {
          output.date = memory.date.trim();
        }
        return output;
      })
      .filter((memory) => memory.text)
  };
}
