// installed by herdr
// managed by herdr; reinstalling or updating the integration overwrites this file.
// add custom hooks/plugins beside this file instead of editing it.
// HERDR_INTEGRATION_ID=opencode
// HERDR_INTEGRATION_VERSION=12

import net from "node:net";

const SOURCE = "herdr:opencode";
const AGENT = "opencode";
const SESSION_DOCTRINE = __HERDR_SESSION_DOCTRINE_JAVASCRIPT__;
let reportSeq = Date.now() * 1000;

// Subagent (task tool) sessions carry a parentID; the main agent session does
// not. Their lifecycle events would otherwise clobber the pane's real state, so
// learn child session ids from session.created/updated and drop their reports.
const childSessions = new Set();

function nextReportSeq() {
  reportSeq += 1;
  return reportSeq;
}

function sessionIDFromProperties(properties) {
  return typeof properties?.sessionID === "string" && properties.sessionID
    ? properties.sessionID
    : undefined;
}

function stateFromSessionStatus(status) {
  // session.status carries { type: "idle" | "busy" | "retry" }; older builds used a bare string.
  const kind = typeof status === "string" ? status : status?.type;
  if (typeof kind !== "string") return undefined;
  switch (kind.toLowerCase()) {
    case "idle":
      return "idle";
    case "active":
    case "busy":
    case "pending":
    case "running":
    case "streaming":
    case "working":
    case "retry":
      return "working";
    default:
      return undefined;
  }
}

function request(method, params) {
  const paneId = process.env.HERDR_PANE_ID;
  const socketPath = process.env.HERDR_SOCKET_PATH;

  if (!paneId || !socketPath) {
    return Promise.resolve();
  }

  const requestId = `${SOURCE}:${Date.now()}:${Math.floor(Math.random() * 1_000_000)
    .toString()
    .padStart(6, "0")}`;
  const request = {
    id: requestId,
    method,
    params: {
      pane_id: paneId,
      source: SOURCE,
      agent: AGENT,
      seq: nextReportSeq(),
      ...params,
    },
  };

  return new Promise((resolve) => {
    const client = net.createConnection(socketPath, () => {
      client.write(`${JSON.stringify(request)}\n`);
    });

    const finish = () => {
      client.destroy();
      resolve();
    };

    client.setTimeout(500, finish);
    client.on("data", finish);
    client.on("error", finish);
    client.on("end", finish);
    client.on("close", resolve);
  });
}

function reportSession(sessionID, sessionStartSource) {
  if (!sessionID) {
    return Promise.resolve();
  }
  const params = { agent_session_id: sessionID };
  if (sessionStartSource) {
    params.session_start_source = sessionStartSource;
  }
  return request("pane.report_agent_session", params);
}

function reportState(state, sessionID) {
  const params = { state };
  if (sessionID) {
    params.agent_session_id = sessionID;
  }
  return request("pane.report_agent", params);
}

// Mail is additive, alongside session/state reporting, and only fires for
// delegated workers: HERDR_PARENT_TERMINAL_ID (authoritative, resolved
// server-side) falls back to HERDR_PARENT_PANE_ID (older spawner/display
// only). Standalone (non-delegated) sessions have neither set, so they stay
// silent by construction.
function parentPaneId() {
  return process.env.HERDR_PARENT_TERMINAL_ID || process.env.HERDR_PARENT_PANE_ID || undefined;
}

function sendMail(to, kind, body) {
  const socketPath = process.env.HERDR_SOCKET_PATH;
  if (!to || !socketPath || !body) {
    return Promise.resolve();
  }

  const requestId = `${SOURCE}:mail:${Date.now()}:${Math.floor(Math.random() * 1_000_000)
    .toString()
    .padStart(6, "0")}`;
  const subject = body.split("\n")[0].slice(0, 120) || "needs attention";
  const request = {
    id: requestId,
    method: "mail.send",
    params: {
      to,
      kind,
      subject,
      body,
      from_pane_id: process.env.HERDR_PANE_ID,
      from_agent: AGENT,
    },
  };

  return new Promise((resolve) => {
    const client = net.createConnection(socketPath, () => {
      client.write(`${JSON.stringify(request)}\n`);
    });

    const finish = () => {
      client.destroy();
      resolve();
    };

    client.setTimeout(500, finish);
    client.on("data", finish);
    client.on("error", finish);
    client.on("end", finish);
    client.on("close", resolve);
  });
}

// Fetch the final assistant message text for a session via the opencode SDK
// client, so `session.idle` can attach a done-mail body without parsing any
// transcript file. Best-effort: any SDK/shape surprise resolves to "" rather
// than throwing, since mail must never break the existing report path.
async function fetchLastAssistantMessage(client, sessionID) {
  if (!client || !sessionID) {
    return "";
  }
  try {
    const response = await client.session.messages({ path: { id: sessionID } });
    const messages = Array.isArray(response) ? response : response?.data;
    if (!Array.isArray(messages)) {
      return "";
    }
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const entry = messages[index];
      if (entry?.info?.role === "assistant") {
        return (entry.parts ?? [])
          .filter((part) => part?.type === "text")
          .map((part) => part.text)
          .join("");
      }
    }
  } catch {
    // best-effort only.
  }
  return "";
}

export const HerdrAgentStatePlugin = async ({ client } = {}) => {
  if (process.env.HERDR_ENV !== "1") {
    return {};
  }

  const hooks = {
    "experimental.chat.system.transform": async (_input, output) => {
      if (!Array.isArray(output?.system)) {
        return;
      }
      if (!output.system.includes(SESSION_DOCTRINE)) {
        output.system.push(SESSION_DOCTRINE);
      }
    },
  };

  if (!process.env.HERDR_SOCKET_PATH || !process.env.HERDR_PANE_ID) {
    return hooks;
  }

  return {
    ...hooks,
    "chat.message": async ({ sessionID }) => {
      if (sessionID && childSessions.has(sessionID)) {
        return;
      }
      await reportState("working", sessionID);
    },
    event: async ({ event }) => {
      const type = event?.type;
      const properties = event?.properties ?? {};
      const sessionID = sessionIDFromProperties(properties);

      const info = properties.info;
      if (info?.id && info.parentID) {
        childSessions.add(info.id);
      }
      if (sessionID && childSessions.has(sessionID)) {
        // Child session events are dropped so they cannot clobber the pane's
        // root-agent state, but a subagent waiting on the user must still
        // surface as blocked (and clear once answered). Report state only,
        // without an agent_session_id, so the pane keeps the root session.
        switch (type) {
          case "permission.asked":
          case "question.asked":
            await reportState("blocked");
            break;
          case "permission.replied":
          case "question.replied":
          case "question.rejected":
            await reportState("working");
            break;
          case "permission.updated": {
            // A sub-agent asking for permission is exactly the "worker needs
            // an answer" signal an orchestrator wants surfaced, so blocked
            // mail is allowed for child sessions too (unlike done-mail,
            // which is root-only).
            const to = parentPaneId();
            if (to) {
              await sendMail(to, "blocked", properties?.title || "needs attention");
            }
            break;
          }
          default:
            break;
        }
        return;
      }

      switch (type) {
        case "session.created":
          // A root session.created is a genuine new-session start (subagent
          // creates are dropped above). Signal it so herdr replaces the pane's
          // prior session id instead of treating the change as cross-talk.
          await reportSession(sessionID, "new");
          break;
        case "session.updated":
          await reportSession(sessionID);
          break;
        case "session.status": {
          const state = stateFromSessionStatus(properties.status);
          if (state) {
            await reportState(state, sessionID);
          } else {
            await reportSession(sessionID);
          }
          break;
        }
        case "tool.execute.before":
        case "tool.execute.after":
        case "permission.replied":
        case "question.replied":
        case "question.rejected":
        case "session.compacted":
          await reportState("working", sessionID);
          break;
        case "permission.asked":
        case "question.asked":
        case "session.error":
          await reportState("blocked", sessionID);
          break;
        case "permission.updated": {
          const blockedTo = parentPaneId();
          if (blockedTo) {
            await sendMail(blockedTo, "blocked", properties?.title || "needs attention");
          }
          break;
        }
        case "session.idle": {
          await reportState("idle", sessionID);
          const doneTo = parentPaneId();
          if (doneTo) {
            const lastMessage = await fetchLastAssistantMessage(client, sessionID);
            if (lastMessage) {
              await sendMail(doneTo, "done", lastMessage);
            }
          }
          break;
        }
        case "session.deleted":
          break;
        default:
          break;
      }
    },
  };
};
