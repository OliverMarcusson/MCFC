# `effect`

```mcfc
entity.effect(name: string, duration: int, amplifier: int) -> void
```

Applies a status effect to the receiver.

```mcfc
fn boost(player: player_ref) -> void:
    player.effect("minecraft:speed", 200, 1)
```

