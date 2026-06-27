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

