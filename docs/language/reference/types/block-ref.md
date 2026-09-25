# `block_ref`

Reference to a block position.

```mcfc
fn place(pos: block_ref) -> void:
    pos.setblock("minecraft:gold_block")
```

## Reading The World

| Method | Returns | Notes |
| --- | --- | --- |
| `pos.light()` | `int` | Light level, 0 to 15, from `location_check` predicates in a binary search |
| `pos.biome()` | `string` | Biome id, such as `"minecraft:plains"`, from `execute if biome` over every biome |
| `pos.in_biome(id)` | `bool` | One `execute if biome`; `id` may be a `#tag` |
| `pos.environment(attribute)` | `float` | A numeric environment attribute from `/compute`. The id must be a literal. |

Numeric environment attributes: `visual/cloud_height`, `visual/fog_start_distance`,
`visual/fog_end_distance`, `visual/sky_fog_end_distance`, `visual/cloud_fog_end_distance`,
`visual/water_fog_start_distance`, `visual/water_fog_end_distance`, `visual/moon_angle`,
`visual/star_angle`, `visual/sun_angle`, `visual/sky_light_factor`, `visual/star_brightness`,
`audio/music_volume`, `gameplay/cat_waking_up_gift_chance`,
`gameplay/creature_world_gen_spawn_probability`, `gameplay/surface_slime_spawn_chance`,
`gameplay/turtle_egg_hatch_chance` and `gameplay/sky_light_level`.

```mcfc
fn report() -> void:
    let here = block("~ ~ ~")
    let light = here.light()
    let biome = here.biome()
    if here.in_biome("#minecraft:is_forest"):
        mcf "say forest, light $(light)"
```

