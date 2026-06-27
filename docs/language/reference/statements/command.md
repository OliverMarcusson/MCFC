# `command`

Declares a Bukkit-style command handler.

```mcfc
command status:
    let player = single(selector("@s"))
    player.tellraw("Ready")
```

Commands always keep the vanilla `/trigger mcfcc_name` fallback. With the agent attached, the same declaration also reserves the real `/name` root command.

