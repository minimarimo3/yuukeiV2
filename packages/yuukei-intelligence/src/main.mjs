import { readFileSync } from "node:fs";
import { loadConfig } from "./config.mjs";
import { capabilityResult, silentOutput } from "./output.mjs";
import { generateWithProvider } from "./providers/index.mjs";

async function main() {
  const invocation = readInvocation();
  if (invocation.capability !== "dialogue.generate") {
    writeResult(capabilityResult(invocation, silentOutput(), { reason: "unsupported-capability" }));
    return;
  }
  const config = await loadConfig();
  const { output, metadata } = await generateWithProvider(invocation.input, config);
  writeResult(capabilityResult(invocation, output, metadata));
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
