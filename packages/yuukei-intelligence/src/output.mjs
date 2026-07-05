export function silentOutput() {
  return { speak: false };
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
  return {
    invocationId: invocation?.id ?? "",
    extensionId: "yuukei-intelligence",
    capability: invocation?.capability ?? "dialogue.generate",
    output: normalizeOutput(output, invocation?.input?.constraints?.maxLength),
    metadata
  };
}
