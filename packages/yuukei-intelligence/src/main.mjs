import { readFileSync } from "node:fs";
import { loadConfig } from "./config.mjs";
import { indexMemory, retrieveMemory } from "./memory.mjs";
import { capabilityResult, silentOutput, unknownChoiceOutput } from "./output.mjs";
import { generateWithProvider, interpretWithProvider } from "./providers/index.mjs";

async function main() {
  const invocation = readInvocation();
  const config = await loadConfig();
  const { output, metadata } = await dispatchInvocation(invocation, config);
  writeResult(capabilityResult(invocation, output, metadata));
}

async function dispatchInvocation(invocation, config) {
  if (invocation.capability === "dialogue.generate") {
    return generateWithProvider(invocation.input, config);
  }
  if (invocation.capability === "dialogue.interpret") {
    return interpretWithProvider(invocation.input, config);
  }
  if (invocation.capability === "memory.index") {
    return indexMemory(invocation.input, config);
  }
  if (invocation.capability === "memory.retrieve") {
    return retrieveMemory(invocation.input);
  }
  return {
    output: invocation.capability === "dialogue.interpret" ? unknownChoiceOutput() : silentOutput(),
    metadata: { reason: "unsupported-capability" }
  };
}

function readInvocation() {
  try {
    return JSON.parse(readFileSync(0, "utf8"));
  } catch (error) {
    console.error(`yuukei-intelligence: failed to read invocation: ${error.message}`);
    return {
      id: "",
      capability: "dialogue.generate",
      input: {}
    };
  }
}

function writeResult(result) {
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

main().catch((error) => {
  console.error(`yuukei-intelligence: unexpected failure: ${error.message}`);
  writeResult(
    capabilityResult(
      { id: "", capability: "dialogue.generate", input: {} },
      silentOutput(),
      { reason: "unexpected-failure" }
    )
  );
});
