# Host Bridge

The host bridge lets a vanilla datapack reach outside Minecraft through an optional companion helper. The datapack exchanges requests and responses through the `mcfc:rpc` command-storage protocol; the generated datapack itself remains vanilla.

Host calls use `module.fn(...)` syntax and suspend like `sleep`.

```mcfc
fn on_join(player: player_ref) -> void:
    let r = http.get("https://api.example.com/motd")
    if r.ok:
        player.tellraw(r.body)
```

Because host calls suspend, they are statement-only. They may appear as a `let` initializer or a bare statement, but not nested inside another expression.

## Manifest Gating

```toml
[helper]
backend = "mcfd"

[helper.capabilities]
http = { allow_domains = ["api.example.com"] }
time = true
rand = true
```

A call to a module that is not enabled in the manifest is a compile error.

## Transport

The default backend is `mcfd`, a standalone external service. Generated datapacks emit a marked command-storage record into Minecraft's `latest.log` by spawning and immediately killing an off-map silent baby pig with a custom name that contains the request.

`mcfd` discovers generated `mcfd.pack.toml` descriptors, tails launcher-specific logs, performs the requested work, and writes results back through a generated inbox function.

::: warning Security model
Capabilities are opt-in and scoped by manifest configuration. HTTP uses domain allowlists, file and KV access use roots, and bearer tokens are referenced by environment variable name rather than stored as secrets in the descriptor.
:::
