#!/bin/bash
# Smoke test of a built agent-eye binary: install, doctor, scan, pick, a hook call, uninstall, and
# check that the agent's settings.json is back to what it was. When it made the Claude Code folder
# itself it then does the same for a machine with only Codex CLI.
#
#   scripts/smoke-install.sh <path-to-agent-eye> [--home <dir>]
#
# It builds nothing. It changes HOME, so it refuses to run unless CI=true (a throw-away runner) or
# you pass --home <dir>, which then becomes HOME for the whole run. It runs on macOS and Linux, and
# never runs launchctl or systemctl (install and uninstall get --no-service).
set -euo pipefail

usage() {
  echo "usage: $0 <path-to-agent-eye> [--home <dir>]" >&2
  echo "  refuses to run unless CI=true or --home <dir> is given" >&2
  exit 2
}

BIN=""
HOME_ARG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --home) [[ $# -ge 2 ]] || usage; HOME_ARG="$2"; shift 2 ;;
    -h|--help) usage ;;
    -*) usage ;;
    *) [[ -z "$BIN" ]] || usage; BIN="$1"; shift ;;
  esac
done
[[ -n "$BIN" ]] || usage
[[ -x "$BIN" ]] || { echo "not an executable: $BIN" >&2; exit 2; }
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

if [[ -n "$HOME_ARG" ]]; then
  mkdir -p "$HOME_ARG"
  NEW_HOME="$(cd "$HOME_ARG" && pwd)"
  # the folder you are logged in with is never a test home
  REAL_HOME="$(cd ~ 2>/dev/null && pwd || true)"
  ACCOUNT_HOME="$(eval echo "~$(id -un)")"
  for real in "$REAL_HOME" "$ACCOUNT_HOME"; do
    if [[ -n "$real" ]] && [[ "$NEW_HOME" = "$(cd "$real" 2>/dev/null && pwd || echo "$real")" ]]; then
      echo "refusing: --home is your real home folder ($NEW_HOME)" >&2
      exit 2
    fi
  done
  export HOME="$NEW_HOME"
elif [[ "${CI:-}" = "true" ]]; then
  : "${HOME:?HOME is not set}"
else
  echo "refusing to run: this changes files under HOME. Set CI=true on a throw-away runner, or pass --home <empty temp dir>." >&2
  exit 2
fi

unset EYE_HOME EYE_CLAUDE_DIR EYE_CLAUDE_BIN CODEX_HOME EYE_CODEX_BIN
export NO_COLOR=1
CLAUDE_DIR="$HOME/.claude"
SETTINGS="$CLAUDE_DIR/settings.json"
INSTALLED="$HOME/.local/bin/agent-eye"
SEEDED=""
# the file that keeps the daemon running, and what is in it
if [[ "$(uname -s)" = "Darwin" ]]; then
  SERVICE_FILE="$HOME/Library/LaunchAgents/dev.agent-eye.daemon.plist"
  SERVICE_TEXT="<string>daemon</string>"
  SERVICE_COMMAND="launchctl bootstrap"
else
  SERVICE_FILE="$HOME/.config/systemd/user/agent-eye.service"
  SERVICE_TEXT="agent-eye daemon"
  SERVICE_COMMAND="systemctl --user enable --now agent-eye.service"
fi

pass() { echo "ok   $*"; }
fail() { echo "FAIL $*" >&2; exit 1; }
contains() { # contains <text> <needle> <what>
  case "$1" in *"$2"*) pass "$3" ;; *) echo "$1" >&2; fail "$3 (missing: $2)" ;; esac
}

# 1. A Claude Code folder to work with: keep the one that is there, else make a small one with a
# foreign hook (which install must leave alone) and one transcript.
if [[ ! -e "$CLAUDE_DIR" ]]; then
  SEEDED=1
  mkdir -p "$CLAUDE_DIR/projects/-smoke"
  cat > "$SETTINGS" <<'JSON'
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/notify/bin/notify stop"
          }
        ]
      }
    ]
  }
}
JSON
  SID="5a0e0000-0000-4000-8000-000000000001"
  {
    printf '{"type":"user","timestamp":"2026-11-01T10:00:00Z","cwd":"/tmp","sessionId":"%s","message":{"content":"Smoke test the zebra importer"}}\n' "$SID"
    printf '{"type":"assistant","timestamp":"2026-11-01T10:00:05Z","cwd":"/tmp","sessionId":"%s","message":{"id":"m1","content":[{"type":"text","text":"Importer checked."}]}}\n' "$SID"
    printf '{"type":"user","timestamp":"2026-11-01T10:01:00Z","cwd":"/tmp","sessionId":"%s","message":{"content":"Thanks"}}\n' "$SID"
  } > "$CLAUDE_DIR/projects/-smoke/$SID.jsonl"
fi
[[ -f "$SETTINGS" ]] || echo '{}' > "$SETTINGS"
BEFORE="$(cat "$SETTINGS")"

# 2. install
out="$("$BIN" install --no-service 2>&1)" || { echo "$out" >&2; fail "install"; }
contains "$out" "hooks added (8 events)" "install adds the hooks"
contains "$out" "not started (--no-service)" "install prints the service commands and does not run them"
contains "$out" "$SERVICE_COMMAND" "the service command is the one of this OS"
[[ -x "$INSTALLED" ]] || fail "binary was not installed at $INSTALLED"
[[ -L "$HOME/.local/bin/eye" ]] || fail "the eye alias was not linked"
[[ -f "$SERVICE_FILE" ]] || fail "service file missing: $SERVICE_FILE"
grep -qF "$SERVICE_TEXT" "$SERVICE_FILE" || fail "the service file does not run the daemon"
[[ -f "$CLAUDE_DIR/skills/eye/SKILL.md" ]] || fail "skill missing"
n="$(grep -c 'agent-eye hook ' "$SETTINGS" || true)"
[[ "$n" -eq 8 ]] || fail "expected 8 hook entries in settings.json, found $n"
if [[ -n "$SEEDED" ]]; then contains "$(cat "$SETTINGS")" "/opt/notify/bin/notify stop" "the other tool's hook is untouched"; fi
pass "install"

# 3. doctor looks and reports; warnings (daemon not running) are fine, a crash is not
set +e
out="$("$INSTALLED" doctor 2>&1)"
code=$?
set -e
[[ "$code" -le 1 ]] || { echo "$out" >&2; fail "doctor exited with $code"; }
contains "$out" "hooks present for all 8 events" "doctor sees the hooks"

# 4. scan and the picker's plain list (no terminal)
out="$("$INSTALLED" scan 2>&1 </dev/null)" || { echo "$out" >&2; fail "scan"; }
contains "$out" "scanned" "scan runs"
if [[ -n "$SEEDED" ]]; then
  contains "$out" "scanned 1 transcripts" "scan finds the transcript"
  out="$("$INSTALLED" pick --filter zebra </dev/null 2>&1)" || { echo "$out" >&2; fail "pick"; }
  contains "$out" "Smoke test the zebra importer" "pick lists the session"
else
  "$INSTALLED" pick --filter smoke </dev/null >/dev/null 2>&1 || fail "pick"
  pass "pick runs"
fi

# 5. a hook call prints nothing and queues one line
out="$(echo '{"session_id":"s1","hook_event_name":"Stop"}' | "$INSTALLED" hook Stop 2>&1)" || fail "hook"
[[ -z "$out" ]] || fail "a hook must print nothing, got: $out"
[[ -s "$HOME/.agent-eye/queue/events.jsonl" ]] || fail "the hook queued nothing"
pass "hook"

# 6. uninstall puts settings.json back exactly as it was
out="$("$INSTALLED" uninstall --no-service 2>&1)" || { echo "$out" >&2; fail "uninstall"; }
AFTER="$(cat "$SETTINGS")"
[[ "$AFTER" = "$BEFORE" ]] || { diff <(echo "$BEFORE") <(echo "$AFTER") >&2 || true; fail "settings.json was not restored"; }
pass "settings.json is restored"
[[ ! -e "$INSTALLED" ]] && [[ ! -e "$HOME/.local/bin/eye" ]] || fail "the binary or alias is still there"
[[ ! -e "$SERVICE_FILE" ]] || fail "the service file is still there"
[[ ! -e "$CLAUDE_DIR/skills/eye" ]] || fail "the skill is still there"
[[ -d "$HOME/.agent-eye" ]] || fail "uninstall must keep the data without --purge"
pass "uninstall"

# 7. a machine with only Codex CLI (only when this script made the Claude folder, so nothing real
# is touched): install follows Codex, adds its hooks beside a foreign one and makes no Claude folder;
# a hook call is queued for Codex; uninstall puts hooks.json back exactly as it was.
if [[ -n "$SEEDED" ]]; then
  rm -rf "$CLAUDE_DIR"
  rm -f "$HOME/.agent-eye/config.toml"
  CODEX_DIR="$HOME/.codex"
  HOOKS="$CODEX_DIR/hooks.json"
  RID="019a7000-0000-7000-8000-000000000001"
  mkdir -p "$CODEX_DIR/sessions/2026/11/01"
  cat > "$HOOKS" <<'JSON'
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/notify/bin/notify stop"
          }
        ]
      }
    ]
  }
}
JSON
  CODEX_BEFORE="$(cat "$HOOKS")"
  {
    printf '{"timestamp":"2026-11-01T10:00:00.000Z","type":"session_meta","payload":{"id":"%s","cwd":"/tmp","source":"cli"}}\n' "$RID"
    printf '{"timestamp":"2026-11-01T10:00:01.000Z","type":"event_msg","payload":{"type":"user_message","message":"Smoke test the codex importer"}}\n'
    printf '{"timestamp":"2026-11-01T10:00:05.000Z","type":"event_msg","payload":{"type":"agent_message","message":"Importer checked."}}\n'
    printf '{"timestamp":"2026-11-01T10:01:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"Thanks"}}\n'
  } > "$CODEX_DIR/sessions/2026/11/01/rollout-2026-11-01T11-00-00-$RID.jsonl"
  out="$("$BIN" install --no-service 2>&1)" || { echo "$out" >&2; fail "codex install"; }
  contains "$out" "agents  Codex CLI (enabled in" "install follows the agent that is there"
  contains "$out" "Next: start Codex and run /hooks once" "install says Codex must trust the hooks"
  [[ ! -e "$CLAUDE_DIR" ]] || fail "a Codex-only machine got a Claude Code folder"
  grep -q 'enabled = \["codex"\]' "$HOME/.agent-eye/config.toml" || fail "config.toml does not enable codex"
  [[ -f "$CODEX_DIR/skills/eye/SKILL.md" ]] || fail "the codex skill is missing"
  n="$(grep -c 'agent-eye hook .* --agent codex' "$HOOKS" || true)"
  [[ "$n" -eq 6 ]] || fail "expected 6 codex hook entries, found $n"
  contains "$(cat "$HOOKS")" "/opt/notify/bin/notify stop" "the other tool's codex hook is untouched"
  set +e
  out="$("$INSTALLED" doctor 2>&1)"
  code=$?
  set -e
  [[ "$code" -le 1 ]] || { echo "$out" >&2; fail "doctor exited with $code"; }
  contains "$out" "hooks present for all 6 events" "doctor sees the codex hooks"
  out="$("$INSTALLED" scan 2>&1 </dev/null)" || { echo "$out" >&2; fail "codex scan"; }
  contains "$out" "scanned 1 transcripts" "scan finds the rollout"
  out="$("$INSTALLED" pick --filter codex </dev/null 2>&1)" || { echo "$out" >&2; fail "codex pick"; }
  contains "$out" "Smoke test the codex importer" "pick lists the codex session"
  out="$(echo "{\"session_id\":\"$RID\",\"hook_event_name\":\"Stop\"}" | "$INSTALLED" hook Stop --agent codex 2>&1)" || fail "codex hook"
  [[ -z "$out" ]] || fail "a hook must print nothing, got: $out"
  grep -q '"agent":"codex"' "$HOME/.agent-eye/queue/events.jsonl" || fail "the hook was not queued for codex"
  out="$("$INSTALLED" uninstall --no-service 2>&1)" || { echo "$out" >&2; fail "codex uninstall"; }
  CODEX_AFTER="$(cat "$HOOKS")"
  [[ "$CODEX_AFTER" = "$CODEX_BEFORE" ]] || { diff <(echo "$CODEX_BEFORE") <(echo "$CODEX_AFTER") >&2 || true; fail "hooks.json was not restored"; }
  [[ ! -e "$CODEX_DIR/skills/eye" ]] || fail "the codex skill is still there"
  pass "codex-only machine"
  rm -rf "$CODEX_DIR"
fi

# 8. tidy up what this script made
if [[ -n "$SEEDED" ]]; then rm -rf "$CLAUDE_DIR"; fi
echo "smoke test passed"
