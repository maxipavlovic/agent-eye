# Sourced by picker-demo.tape (zsh), from the repository root, after `cargo build --release`.
# Builds a made-up Claude Code folder and points the binary at it, so no real data is involved.
DEMO_DIR="${DEMO_DIR:-/tmp/agent-eye-demo}"
sleep 7200 >/dev/null 2>&1 &! # stands in for the open session's process
node scripts/demo-home.mjs "$DEMO_DIR" $! >/dev/null
export HOME="$DEMO_DIR/home" EYE_HOME="$DEMO_DIR/eye" EYE_CLAUDE_DIR="$DEMO_DIR/claude"
# The fresh build as `agent-eye` and `eye`, and a stand-in `claude` that answers the summary calls.
ln -s "$PWD/target/release/agent-eye" "$DEMO_DIR/bin/agent-eye"
ln -s agent-eye "$DEMO_DIR/bin/eye"
export PATH="$DEMO_DIR/bin:$PATH"
eye scan >/dev/null
eye summarize --all >/dev/null
eval "$(eye init zsh)"
PROMPT='%F{cyan}~/code%f $ '
