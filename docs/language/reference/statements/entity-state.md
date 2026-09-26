# `entity_state`

Declares typed state for an `entity_ref`. Entity state declarations are top-level and accept `int`, `bool`, `string`, `float`, and named struct types. Unlike `player_state`, an `entity_state` declaration has no display name.

```mcfc
struct MarkerInfo:
    label: string
    weight: float

entity_state info: MarkerInfo

fn mark(entity: entity_ref) -> void:
    entity.state.info = MarkerInfo { label: "Target", weight: 1.5 }
    debug(entity.state.info.label)
```

Access a declaration through `entity.state.name`; you can read or write the whole struct or its fields. Assignments must match the declared type. Undeclared `entity.state.*` paths remain available for `int` and `bool`; string, float, and struct state require a declaration.

## Under The Hood

`int` and `bool` entity state use MCFC-managed scoreboard objectives with the `mcfe_*` prefix. String, float, and struct state live in `<namespace>:state` command storage keyed by the entity's four-part UUID. Missing values read as `""`, `0.0`, or `{}` respectively. Setup does not clear stored values; values can remain after an entity despawns.
