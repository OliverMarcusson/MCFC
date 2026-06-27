# `match`

Branches on string arms with an optional `else` fallback.

```mcfc
fn handle(action: string) -> void:
    match action:
        "jump" => debug("leap")
        "pathfind" => debug("move")
        else => debug("idle")
```

Duplicate string arms are rejected.

## Under The Hood

`match` lowers to string comparisons against generated storage values. Matching arms are emitted as guarded generated block functions, with an `else` arm used when no earlier arm matched.
