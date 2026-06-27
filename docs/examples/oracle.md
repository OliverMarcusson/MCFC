# Oracle

`examples/oracle` is a showcase of a vanilla datapack talking to the real world through `mcfd`.

It fetches random number facts from `numbersapi.com`, reads real-world time, tracks a per-player prophecy count, and exposes a roll function using helper randomness.

## Build

```powershell
cargo build -p mcfd --release
cargo run --bin mcfc -- build examples/oracle/mcfc.toml --out examples/oracle/dist
```

## Run

1. Copy `examples/oracle/dist/` into the world's `datapacks/` directory.
2. Install the standalone helper once with `mcfd service install`.
3. Load the world or run `/reload`.
4. Try `/function oracle:roll`.

If `mcfd` is not running, host calls time out and the Oracle reports that it is silent.
