// agent-eye plugin for OpenCode. Written by `agent-eye install`, removed by `agent-eye uninstall`;
// edits are overwritten. It only watches: for a few events of a session (a turn starts, a
// permission is asked or answered, a turn ends) it hands one small JSON payload to
// `agent-eye hook <Event> --agent opencode`, which appends one line to eye's queue. It never
// changes what OpenCode does, never waits for eye, and says nothing when something goes wrong.
// agent-eye-plugin: 1
import { spawn } from "node:child_process"

// install writes the absolute path of agent-eye here
const EYE = "/__EYE_BIN__"
const KEY = Symbol.for("agent-eye.opencode.plugin")

// one hook at a time, in the order the events came: a permission answered must not be queued
// before it was asked
let chain = Promise.resolve()
function send(event, payload) {
  if (!EYE.startsWith("/")) return
  chain = chain
    .then(
      () =>
        new Promise((resolve) => {
          try {
            // EYE is the absolute path install wrote in; the hook needs nothing from PATH, so it gets a fixed one
            const child = spawn(EYE, ["hook", event, "--agent", "opencode"], {
              stdio: ["pipe", "ignore", "ignore"],
              env: { ...process.env, PATH: "/usr/bin:/bin" },
            })
            const timer = setTimeout(() => {
              try {
                child.kill()
              } catch {}
              resolve()
            }, 5000)
            child.on("error", () => (clearTimeout(timer), resolve()))
            child.on("close", () => (clearTimeout(timer), resolve()))
            child.stdin.on("error", () => {})
            child.stdin.end(JSON.stringify(payload))
          } catch {
            resolve()
          }
        }),
    )
    .catch(() => {})
}

const text = (v) => (typeof v === "string" ? v : "")

export default {
  id: "agent-eye",
  setup: async (ctx) => {
    // OpenCode may set a plugin up more than once in a process: watch once
    if (globalThis[KEY]) return
    globalThis[KEY] = true
    const last = new Map() // session id -> text of its newest reply
    const seen = new Set() // event ids already handled
    let iterator
    let stopped = false
    const handle = (ev) => {
      const d = ev && ev.data
      if (!d || !d.sessionID || seen.has(ev.id)) return
      seen.add(ev.id)
      if (seen.size > 500) seen.delete(seen.values().next().value)
      const base = { session_id: d.sessionID }
      switch (ev.type) {
        case "session.created":
          send("SessionStart", {
            ...base,
            cwd: text(d.location && d.location.directory) || text(ev.location && ev.location.directory),
          })
          break
        case "session.execution.started":
          last.delete(d.sessionID)
          send("UserPromptSubmit", base)
          break
        case "session.text.ended":
          if (text(d.text)) last.set(d.sessionID, text(d.text))
          break
        case "permission.asked":
          send("PermissionRequest", {
            ...base,
            tool_name: text(d.action),
            resources: Array.isArray(d.resources) ? d.resources.map(text).slice(0, 8) : [],
          })
          break
        case "permission.replied":
          send("PermissionReplied", { ...base, reply: text(d.reply) })
          break
        case "session.execution.succeeded":
        case "session.execution.failed":
        case "session.execution.interrupted": {
          const reason = ev.type.slice("session.execution.".length)
          const tail = last.get(d.sessionID) || ""
          last.delete(d.sessionID)
          send("Stop", { ...base, reason, last_assistant_message: reason === "succeeded" ? tail.slice(-400) : "" })
          break
        }
      }
    }
    ;(async () => {
      try {
        iterator = ctx.event.subscribe()[Symbol.asyncIterator]()
        while (!stopped) {
          const next = await iterator.next()
          if (next.done) break
          try {
            handle(next.value)
          } catch {}
        }
      } catch {}
    })().catch(() => {})
    return () => {
      stopped = true
      globalThis[KEY] = false
      try {
        iterator && iterator.return && iterator.return()
      } catch {}
    }
  },
}
