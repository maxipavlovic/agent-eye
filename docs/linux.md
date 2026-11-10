# Linux

agent-eye works the same on Linux, with these differences:

- **The background process is a systemd user service.** `agent-eye install` writes `~/.config/systemd/user/agent-eye.service` and starts it with `systemctl --user`; its log is `journalctl --user -u agent-eye`. It runs while you are logged in; `loginctl enable-linger $USER` keeps it running after you log out. Where there is no systemd, run `agent-eye daemon` yourself (`install --no-service` skips the service).
- **Resume and Start copy the command** instead of opening a terminal tab (`[board] resume = "copy"` is the default). In the picker, Enter and Ctrl-G run the command in your current terminal as usual.
- **Copying** uses `wl-copy`, `xclip` or `xsel`, the first that works. With none, the command is printed instead. The background process copies for the board, and it may have no display; then the board shows the command.
- `eye board on --open` opens the board with `xdg-open`.
