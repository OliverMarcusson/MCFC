# `command`

Declares a Bukkit-style command handler.

```mcfc
command status:
    let player = single(selector("@s"))
    player.tellraw("Ready")
```

Commands always keep the vanilla `/trigger mcfcc_name` fallback. With the agent attached, the same declaration also reserves the real `/name` root command.

## Under The Hood

MCFC creates a trigger objective for the vanilla fallback and checks it from the generated Bukkit tick dispatcher. Agent-enabled packs also get an `agent/command/<name>.mcfunction` wrapper for real root-command dispatch.
