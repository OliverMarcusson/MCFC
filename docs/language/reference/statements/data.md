# `data`

Declares a Bukkit-style scoreboard-backed player data field.

```mcfc
data player.coins: int = 0
data player.ready: bool = false
```

The field is accessed through `player.data.name` or the corresponding state surface.

## Under The Hood

`data player.*` declares scoreboard-backed player state. MCFC creates the needed objective during generated setup and compiles reads/writes to `scoreboard players get`, `scoreboard players set`, or scoreboard operations.
