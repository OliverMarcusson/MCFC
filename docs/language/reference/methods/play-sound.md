# `play_sound`

```mcfc
entity.play_sound(sound: string, category: string) -> void
```

Bukkit-style alias for [`playsound`](./playsound).

```mcfc
fn chime(player: player_ref) -> void:
    player.play_sound("minecraft:entity.player.levelup", "master")
```

