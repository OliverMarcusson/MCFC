# Named Struct Types

User-defined structured types declared with `struct`.

```mcfc
struct Reward:
    name: string
    amount: int

fn give(reward: Reward) -> void:
    debug(reward.name)
```

