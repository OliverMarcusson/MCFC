# Cyber Quotes

An mcfc/mcfd example that broadcasts a random Cyber quote from Munin using
Minecraft text components. Run `/function cyber_quotes:quote` in game.

## Setup

Put the Munin bearer secret in `assets/.env`. The build copies that file into the
datapack root, beside `mcfd.pack.toml`, where `mcfd` loads it for this datapack
only. Do not add the token to the manifest or repository.

```dotenv
MUNIN_EVENTS_API_SECRET=<your Munin events API secret>
```

`assets/.env.example` is the tracked template. The real `assets/.env` is ignored
by Git. Rebuild and redeploy the datapack after changing it; no Windows-level
environment-variable setup or service restart is needed.

## Build and run

```powershell
cargo build -p mcfd --release
cargo run --bin mcfc -- build examples/cyber_quotes/mcfc.toml --out examples/cyber_quotes/dist
```

Copy `dist/` into the world's `datapacks/` directory, then install/start the
helper once with `mcfd service install`. The generated `mcfd.pack.toml` contains
only the environment-variable name, never the secret value.
Each host request reaches `mcfd` through an immediately killed, off-map pig death
record; it does not use chat or admin-command logging.

The datapack calls Munin's random Cyber quote endpoint once, extracts the quote
text, author, and source from that same JSON response, and broadcasts a styled
component to every online player. A styled fallback appears when the helper,
token, endpoint, or quote is unavailable.
