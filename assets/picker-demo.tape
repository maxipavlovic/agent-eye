# Records assets/picker-demo.gif with VHS (https://github.com/charmbracelet/vhs).
# From the repository root, after `cargo build --release`:  vhs assets/picker-demo.tape
# The sessions are made up: assets/demo/setup.sh builds them in a temp folder.
Output assets/picker-demo.gif

Set Shell "zsh"
Set FontFamily "Menlo"
Set FontSize 15
Set Width 1120
Set Height 520
Set Padding 18
Set Framerate 20
Set WindowBar Colorful
Set TypingSpeed 85ms
Set CursorBlink false
Set Theme { "name": "agent-eye", "background": "#1e1d1a", "foreground": "#d6d1c4", "cursor": "#e8683f", "selection": "#3a3833", "black": "#1e1d1a", "red": "#e8683f", "green": "#9bbf7a", "yellow": "#d9b45a", "blue": "#7ea6cf", "magenta": "#b48ead", "cyan": "#d6d1c4", "white": "#d6d1c4", "brightBlack": "#7d796e", "brightRed": "#ff4a1c", "brightGreen": "#9bbf7a", "brightYellow": "#d9b45a", "brightBlue": "#7ea6cf", "brightMagenta": "#b48ead", "brightCyan": "#d6d1c4", "brightWhite": "#f2efe6" }

Hide
Type "source assets/demo/setup.sh && clear"
Enter
Sleep 6s
Show

# The picker: the sessions that need you come first.
Type "eye"
Sleep 500ms
Enter
Sleep 2500ms
Down 2
Sleep 800ms

# Search, preview, conversation.
Type "webhook"
Sleep 2500ms
Ctrl+T
Sleep 3s
Ctrl+T
Sleep 800ms
Ctrl+U
Type "queue"
Sleep 2s
Down
Sleep 1500ms
Escape
Sleep 1s

# The shell key: type a word, press Ctrl-G, and the resume command lands on your prompt.
Type "flaky"
Sleep 500ms
Ctrl+G
Sleep 2s
Enter
Sleep 3s
