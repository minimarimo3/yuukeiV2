// Codex app-server への実装委任ブリッジ。使い方は .claude/skills/codex/SKILL.md 参照。
//   node tools/codex-bridge.mjs <仕様書ファイル> [再開threadId]
// 前提: codex app-server --listen ws://127.0.0.1:4500 が起動中。Node 22+(グローバルWebSocket)。
// env: BRIDGE_TIMEOUT_MIN(既定45) / BRIDGE_EFFORT(既定medium)
import { readFileSync, appendFileSync } from "node:fs";

const WS_URL = "ws://127.0.0.1:4500";
const PROJECT = "/Users/minimarimo/Sagyouba/V2/Yuukei";
const MODEL = "gpt-5.5";
const promptFile = process.argv[2];
if (!promptFile) {
  console.error("usage: node codex-bridge.mjs <prompt-file>");
  process.exit(2);
}
const prompt = readFileSync(promptFile, "utf8");

const SAFE_COMMAND =
  /^(cargo (build|check|test|fmt|clippy|metadata)|pnpm (install|run|test|exec|-r|--filter)|npx tsc|node |ls|rg |grep |cat |head |tail |find |git (status|diff|log|show|ls-files)|sed -n|wc )/;

let nextId = 1;
const pending = new Map();
let threadId = null;
const deadline = setTimeout(() => {
  console.error("TIMEOUT");
  process.exit(1);
}, Number(process.env.BRIDGE_TIMEOUT_MIN ?? 45) * 60 * 1000);

const ws = new WebSocket(WS_URL);
function send(obj) {
  ws.send(JSON.stringify(obj));
}
function request(method, params) {
  return new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    send({ method, id, params });
  });
}
function short(obj, n = 600) {
  const s = JSON.stringify(obj);
  return s.length > n ? s.slice(0, n) + "…" : s;
}

ws.onerror = (e) => {
  console.error("WS ERROR", e.message ?? e);
  process.exit(1);
};

ws.onopen = async () => {
  try {
    await request("initialize", {
      clientInfo: {
        name: "fable_codex_bridge",
        title: "Fable Codex Bridge",
        version: "0.1.0",
      },
    });
    send({ method: "initialized", params: {} });
    const resumeId = process.argv[3];
    const t = resumeId
      ? await request("thread/resume", { threadId: resumeId })
      : await request("thread/start", {
          model: MODEL,
          cwd: PROJECT,
          approvalPolicy: "on-request",
          sandbox: "workspace-write",
          serviceName: "fable_codex_bridge",
        });
    threadId = t.thread?.id ?? t.threadId ?? resumeId;
    console.log("THREAD", threadId);
    await request("turn/start", {
      threadId,
      cwd: PROJECT,
      approvalPolicy: "on-request",
      sandboxPolicy: {
        type: "workspaceWrite",
        writableRoots: [PROJECT],
        networkAccess: false,
      },
      model: MODEL,
      effort: process.env.BRIDGE_EFFORT ?? "medium",
      summary: "concise",
      input: [{ type: "text", text: prompt }],
    });
    console.log("TURN_START_ACCEPTED");
  } catch (err) {
    console.error("SETUP FAILED", err.message ?? err);
    process.exit(1);
  }
};

function decideApproval(method, params) {
  const blob = JSON.stringify(params ?? {});
  if (method.includes("fileChange")) {
    if (!blob.includes("..") ) return { decision: "approved", why: "file change in workspace sandbox" };
  }
  const cmd =
    params?.item?.command ??
    params?.command ??
    params?.item?.commandLine ??
    "";
  let cmdStr = Array.isArray(cmd) ? cmd.join(" ") : String(cmd);
  cmdStr = cmdStr.replace(/^\/bin\/(zsh|bash|sh) -lc ['"]?/, "");
  if (method.includes("commandExecution") || cmdStr) {
    if (SAFE_COMMAND.test(cmdStr))
      return { decision: "approved", why: `safe command: ${cmdStr.slice(0, 120)}` };
    return { decision: "denied", why: `not on safe list: ${cmdStr.slice(0, 200)}` };
  }
  return { decision: "denied", why: `unknown approval type ${method}` };
}

ws.onmessage = (ev) => {
  let msg;
  try {
    msg = JSON.parse(ev.data);
  } catch {
    console.error("BAD FRAME", String(ev.data).slice(0, 200));
    return;
  }
  if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined)) {
    const p = pending.get(msg.id);
    if (p) {
      pending.delete(msg.id);
      msg.error ? p.reject(new Error(short(msg.error))) : p.resolve(msg.result);
    }
    return;
  }
  if (msg.id !== undefined && msg.method) {
    const { decision, why } = decideApproval(msg.method, msg.params);
    console.log(`APPROVAL ${msg.method} -> ${decision} (${why})`);
    if (decision === "denied") console.log("APPROVAL_PARAMS", short(msg.params, 800));
    send({ id: msg.id, result: { decision } });
    return;
  }
  handleNotification(msg);
};

function handleNotification(msg) {
  const m = msg.method ?? "?";
  const p = msg.params ?? {};
  switch (m) {
    case "turn/started":
      console.log("TURN_STARTED");
      break;
    case "turn/plan/updated": {
      const steps = p.plan?.steps ?? p.plan ?? p;
      console.log("PLAN", short(steps, 500));
      break;
    }
    case "item/started": {
      const t = p.item?.type ?? p.item?.itemType ?? "?";
      console.log("ITEM_STARTED", t);
      break;
    }
    case "item/completed": {
      const item = p.item ?? {};
      const t = item.type ?? item.itemType ?? "?";
      if (t === "agentMessage") {
        console.log("AGENT_MESSAGE_BEGIN");
        console.log(item.text ?? item.content ?? short(item, 2000));
        console.log("AGENT_MESSAGE_END");
      } else if (t === "commandExecution") {
        const cmd = Array.isArray(item.command) ? item.command.join(" ") : item.command;
        console.log("CMD_DONE", `exit=${item.exitCode ?? item.exit_code ?? "?"}`, String(cmd).slice(0, 200));
      } else if (t === "fileChange") {
        const files = (item.changes ?? item.files ?? []).map((c) => c.path ?? c).slice(0, 30);
        console.log("FILE_CHANGE", JSON.stringify(files));
      } else if (t === "reasoning" || t === "plan") {
        console.log("ITEM_DONE", t);
      } else {
        console.log("ITEM_DONE", t, short(item, 300));
      }
      break;
    }
    case "item/agentMessage/delta":
      break;
    case "turn/diff/updated":
      break;
    case "serverRequest/resolved":
      break;
    case "turn/completed": {
      console.log("TURN_COMPLETED", short(p, 1500));
      clearTimeout(deadline);
      const status = p.turn?.status ?? p.status ?? "unknown";
      appendFileSync(new URL("./codex-thread-id.txt", import.meta.url), `${threadId}\n`);
      setTimeout(() => process.exit(status === "failed" ? 1 : 0), 200);
      break;
    }
    default:
      console.log("NOTIF", m, short(p, 200));
  }
}
