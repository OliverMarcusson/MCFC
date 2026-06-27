# mcfd-agent

Optional Java instrumentation agent for MCFC. It is not a mod or a Bukkit/Paper
plugin. `mcfd-agent.jar` can be attached dynamically to a running Minecraft JVM;
the companion attach launcher uses the JDK Attach API.

Build it on Windows with:

```powershell
.\mcfd-agent\build.ps1
```

Verify the named 26.1.2 mappings against a Minecraft JAR before changing hook
targets:

```powershell
.\mcfd-agent\verify-26.1.2.ps1
```

Run the reflection dispatch self-test (event storage/function dispatch plus
root-command routing) with:

```powershell
.\mcfd-agent\test.ps1
```

The current 26.1.2 adapter instruments named vanilla server methods for chat,
inventory, interaction, lifecycle, player-state, and item events. It emits a
human-readable `[mcfd-agent] event=...` line followed by a versioned JSON record
that mcfd parses. Subscribed MCFC event handlers are then invoked on the server
thread as the affected player; declared no-argument commands also receive real
root-command routes.

Cancellable packet-entry hooks (for example `chat`, `inventory_click`,
`player_action`, and `block_break`) can be cancelled by calling `event.cancel()`
inside a typed MCFC handler before vanilla handles them. Lifecycle hooks such as
`player_damage`, `player_teleport`, `player_connect`, and `player_quit` are
observation-only and reject cancellation.

The adapter is deliberately pinned to Minecraft 26.1.2. Restart Minecraft
after updating the agent so mcfd attaches the new JAR with the current pack
subscriptions.
