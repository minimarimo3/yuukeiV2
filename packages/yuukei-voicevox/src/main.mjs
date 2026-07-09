import { readFileSync } from "node:fs";
import {
  loadConfig,
  synthesizeWithVoicevox,
  voicevoxCapabilityResult
} from "./voicevox.mjs";

async function main() {
  const invocation = readInvocation();
  const config = loadConfig();
  const output = await synthesizeWithVoicevox(invocation, config);
  writeResult(voicevoxCapabilityResult(invocation, output));
}

function readInvocation() {
  try {
    return JSON.parse(readFileSync(0, "utf8"));
  } catch (error) {
    console.error(`yuukei-voicevox: failed to read invocation: ${error.message}`);
    return {
      id: "",
      capability: "speech.synthesis",
      input: {}
    };
  }
}

function writeResult(result) {
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

main().catch((error) => {
  console.error(`yuukei-voicevox: synthesis failed: ${error.message}`);
  process.exitCode = 1;
});
