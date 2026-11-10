# Linux

agent-eye works the same on Linux, with these differences:

- **The background process is a systemd user service.** `agent-eye install` writes `~/.config/systemd/user/agent-eye.service` and starts it with `systemctl --user`; its log is `journalctl --user -u agent-eye`. It runs while you are logged in; `loginctl enable-linger $USER` keeps it running after you log out. Where there is no systemd, run `agent-eye daemon` yourself (`install --no-service` skips the service).
- **Resume and Start open a terminal window** when there is a display (`DISPLAY` or `WAYLAND_DISPLAY`), with the first of `xdg-terminal-exec`, `x-terminal-emulator` and `$TERMINAL` that is installed (`[board] resume = "auto"`, the default). The board says which: `TERMINAL (xdg-terminal-exec)`. With no display or no such program they copy the command instead, and the board says `COPY`. `resume = "copy"` always copies; `resume = "terminal"` skips the display check. In the picker, Enter and Ctrl-G run the command in your current terminal as usual.
- **The background service may have no display.** A systemd user service only gets `DISPLAY` or `WAYLAND_DISPLAY` when your session passes them on (`systemctl --user import-environment DISPLAY WAYLAND_DISPLAY`, which most desktops do at login). Without them the board copies. A terminal that the service opens is a child of the service, so restarting the service (for example `agent-eye install` on a new binary) closes it.
- **Copying** uses `wl-copy`, `xclip` or `xsel`, the first that works. With none, the command is printed instead. The background process copies for the board, and it may have no display; then the board shows the command.
- `eye board on --open` opens the board with `xdg-open`.
