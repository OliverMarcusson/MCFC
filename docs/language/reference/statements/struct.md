# `struct`

Declares a named struct type with typed fields. Struct declarations are top-level and use an indented field list.

```mcfc
struct Quest:
    name: string
    reward: int

fn show(quest: Quest) -> void:
    debug(quest.name)
```

Fields are accessed with path syntax such as `quest.reward`.

