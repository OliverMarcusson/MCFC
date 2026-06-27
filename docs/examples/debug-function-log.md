# Debug Function Log

`examples/debug_function_log` is a minimal plain-vanilla datapack that validates MCFC's log transport pattern.

It runs a function from Minecraft's `load` tag, stores a sample RPC envelope, gives an off-map baby pig a custom name that renders that storage value, and immediately kills it.

## Build

This example needs no compilation. Copy `examples/debug_function_log/datapack/` into a world's `datapacks/` directory and run `/reload`.

## Verify

Inspect `logs/latest.log` for a pig death line containing the marker and envelope:

```text
[mcfc_rpc] {mcpipe:1b,protocol:2,pack:"debug_function_log",id:7,mod:"mcfd",fn:"ping",args:[]}
```

Minecraft may append normal death-message text after the compound. The important check is that the macro-expanded custom name contains the storage value.
