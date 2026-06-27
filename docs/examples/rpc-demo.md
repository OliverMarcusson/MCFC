# RPC Demo

`examples/rpc_demo` is the smallest host-bridge example. It performs an HTTP request through `mcfd`.

## Build

```powershell
cargo build -p mcfd --release
cargo run --bin mcfc -- build examples/rpc_demo/mcfc.toml --out examples/rpc_demo/dist
```

## Run

1. Drop `examples/rpc_demo/dist/` into the world's `datapacks/` directory.
2. Install the global helper once with `mcfd service install`.
3. Load the world or run `/reload`.

Only `api.example.com` is allow-listed in `mcfc.toml`; requests to other domains are rejected by the helper.
