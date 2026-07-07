import { readFileSync } from "node:fs";
import { loadConfig } from "./config.mjs";
import { forgetMemory, indexMemory, listMemory, retrieveMemory, updateMemory } from "./memory.mjs";
import { capabilityResult, silentOutput, unknownChoiceOutput, unknownExtractOutput } from "./output.mjs";
import { evaluateMoodWithProvider, extractWithProvider, generateWithProvider, interpretWithProvider } from "./providers/index.mjs";

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
  if (invocation.capability === "dialogue.extract") {
    return extractWithProvider(invocation.input, config);
  }
  if (invocation.capability === "memory.index") {
    return indexMemory(invocation.input, config);
  }
  if (invocation.capability === "memory.list") {
    return listMemory(invocation.input);
  }
  if (invocation.capability === "memory.retrieve") {
    return retrieveMemory(invocation.input);
  }
  if (invocation.capability === "memory.update") {
    return updateMemory(invocation.input);
  }
  if (invocation.capability === "memory.forget") {
    return forgetMemory(invocation.input);
  }
  if (invocation.capability === "mood.evaluate") {
    return evaluateMoodWithProvider(invocation.input, config);
  }
  return {
    output:
      invocation.capability === "dialogue.interpret"
        ? unknownChoiceOutput()
        : invocation.capability === "dialogue.extract"
          ? unknownExtractOutput()
          : silentOutput(),
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
