# Third-party licenses

agent-eye itself is MIT (see `../LICENSE`). The web board embeds three fonts under the SIL Open Font License 1.1. Their license texts are kept here because the license asks that every copy of the font carries it.

| File | Font | Used for | Copyright |
|---|---|---|---|
| `OFL-Inter.txt` | Inter (variable) | board text | The Inter Project Authors, https://github.com/rsms/inter |
| `OFL-Silkscreen.txt` | Silkscreen (regular, bold) | labels and column titles | The Silkscreen Project Authors, https://github.com/googlefonts/silkscreen |
| `OFL-Doto.txt` | Doto (800) | counts | The Doto Project Authors, https://github.com/oliverlalan/Doto |

The fonts come from the `@fontsource` npm packages and are bundled unmodified by `face/` at build time. They are not committed to this repository.

Rust and npm dependencies keep their own licenses. `deny.toml` lists the Rust licenses the build accepts and CI checks them with `cargo deny check`.
