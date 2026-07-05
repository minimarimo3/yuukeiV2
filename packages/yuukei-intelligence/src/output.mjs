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
  try {
    return normalizeOutput(JSON.parse(text), maxLength);
  } catch (error) {
    console.error(`yuukei-intelligence: failed to parse provider JSON: ${error.message}`);
    return silentOutput();
  }
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
