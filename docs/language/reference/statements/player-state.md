# `player_state`

Declares a typed value accessed through `player.state.name`. State declarations are top-level. Supported types are `int`, `bool`, `string`, `float`, and named structs.

```mcfc
struct Profile:
    level: int
    title: string

player_state profile: Profile = "Profile"
player_state nickname: string = "Nickname"

fn update(player: player_ref) -> void:
    player.state.profile = Profile { level: 3, title: "Scout" }
    player.state.nickname = "Alex"
    debug(player.state.profile.title)
```

The string after `=` is the display name for scoreboard-backed `int` and `bool` state. String, float, and struct state use command storage and do not create a scoreboard objective. Values assigned to a declared state must have its declared type. You can read or write a whole struct, or a field such as `player.state.profile.level`.

Undeclared `player.state.*` paths remain available for `int` and `bool` values. String, float, and struct state require a declaration.

## Under The Hood

`int` and `bool` state live in MCFC-managed scoreboard objectives with the `mcfs_*` prefix. String, float, and struct state live in `<namespace>:state` command storage under a key derived from the player's four-part UUID. Missing values read as `""`, `0.0`, or `{}` respectively. Stored values persist across datapack reloads; setup does not clear them.
