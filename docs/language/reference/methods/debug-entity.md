# `debug_entity`

```mcfc
entity.debug_entity(label: string) -> void
```

Emits debug information for the receiver.

```mcfc
fn inspect() -> void:
    let target = single(selector("@e[limit=1]"))
    target.debug_entity("target")
```

