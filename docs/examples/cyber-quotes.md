# Cyber Quotes

`examples/cyber_quotes` broadcasts a random Cyber quote from Munin using Minecraft text components.

## Secret Setup

Put the bearer secret in `assets/.env`:

```dotenv
MUNIN_EVENTS_API_SECRET=<your Munin events API secret>
```

The build copies that file into the datapack root beside `mcfd.pack.toml`, where `mcfd` loads it for this datapack only. Do not put the token in the manifest or repository.

## Build

```powershell
cargo build -p mcfd --release
cargo run --bin mcfc -- build examples/cyber_quotes/mcfc.toml --out examples/cyber_quotes/dist
```

## Run

Copy `dist/` into the world's `datapacks/` directory, then install or start the helper with `mcfd service install`. Run:

```text
/function cyber_quotes:quote
```

The generated descriptor contains only the environment-variable name, never the secret value.
