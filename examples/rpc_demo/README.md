# rpc_demo

Minimal example of the mcfc **host bridge**: a vanilla datapack that performs an
HTTP request through the `mcfd` helper.

## Build

```powershell
cargo build -p mcfd --release
cargo run --bin mcfc -- build examples/rpc_demo/mcfc.toml --out examples/rpc_demo/dist
```

This writes the datapack to `dist/`, emits `dist/mcfd.toml`, and copies the `mcfd`
binary next to it.

## Run

1. Drop `dist/` into your world's `datapacks/` folder.
2. Run the helper from the `dist/` folder: `./mcfd mcfd.toml` (the log path is
   auto-detected from the datapack location).
4. In game, run the `main` function (or `/reload` then trigger it). The datapack
   prints a `[mcfc_rpc]` marker, `mcfd` performs the request, and the result is
   injected back so the follow-up `tellraw` fires.

Only `api.example.com` is allow-listed in `mcfc.toml`; requests to other domains are
rejected by the helper.
