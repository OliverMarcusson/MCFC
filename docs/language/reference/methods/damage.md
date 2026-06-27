# `damage`

```mcfc
entity.damage(amount: int) -> void
```

Applies damage to the receiver.

```mcfc
fn hurt_nearest() -> void:
    let pig = single(selector("@e[type=minecraft:pig,limit=1]"))
    pig.damage(2)
```

