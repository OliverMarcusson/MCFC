# Debug Function Log Probe

This minimal plain-vanilla datapack validates MCFC's VanilLog-style log transport.
It runs one function directly from Minecraft's `load` tag, stores a sample RPC
envelope, gives an off-map baby pig a custom name that renders that storage value,
then immediately kills the pig.

## Build

Copy [`datapack/`](datapack/) into a world's `datapacks/` directory, then run
`/reload`. It needs no compilation.

## Run

Reloading the datapack runs `debug_function_log:emit` automatically. Inspect
`logs/latest.log` for a pig death line containing the full marker and envelope:

```text
[mcfc_rpc] {mcpipe:1b,protocol:2,pack:"debug_function_log",id:7,mod:"mcfd",fn:"ping",args:[]}
```

Minecraft may append the normal death-message text after the compound. The
important check is that the macro-expanded custom name contains the storage value
before the message reaches the log.
