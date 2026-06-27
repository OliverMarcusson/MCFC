# mcfd-agent

`mcfd-agent` is an optional Java instrumentation agent. It is not a mod and not a Bukkit/Paper plugin. It can be attached dynamically to a running Minecraft JVM through the JDK Attach API.

## Build

```powershell
.\mcfd-agent\build.ps1
```

## Verify Mappings

Before changing hook targets, verify named 26.2 mappings against a Minecraft JAR:

```powershell
.\mcfd-agent\verify-26.2.ps1
```

## Self-test

Run the reflection dispatch self-test:

```powershell
.\mcfd-agent\test.ps1
```

## Runtime Behavior

The adapter instruments named vanilla server methods for chat, inventory, interaction, lifecycle, player-state, and item events. It emits a human-readable `[mcfd-agent] event=...` line followed by a versioned JSON record that `mcfd` parses.

Subscribed MCFC event handlers are invoked on the server thread as the affected player. Declared no-argument commands can also receive real root-command routes.

::: warning Version pin
The adapter is deliberately pinned to Minecraft `26.2`. Restart Minecraft after updating the agent so `mcfd` attaches the new JAR with the current pack subscriptions.
:::
