# `switch`

Branches on an enum, `int`, or `string` value. Each `case` has an indented body with one or more statements; `default` handles values without a matching case.

```mcfc
enum QuestStage:
    NEW
    ACTIVE
    COMPLETE

fn announce(stage: QuestStage) -> void:
    switch stage:
        case QuestStage.NEW:
            debug("Quest available")
        case QuestStage.ACTIVE:
            debug("Quest underway")
            debug("Keep going")
        case QuestStage.COMPLETE:
            debug("Quest complete")
```

Cases must be constants of the switch value's type: enum constants, integer literals, or string literals. Duplicate cases and duplicate `default` arms are rejected. An enum switch must cover every constant unless it has a `default` arm. Integer and string switches may omit `default`.

`switch` evaluates its value once and runs the first matching body. Each case has its own local scope. The existing [`match`](./match) statement remains available for compact string comparisons with one statement per arm.

## Under The Hood

The compiler stores the switch value in a temporary slot and lowers the arms to nested `if` branches. Enum and integer comparisons use scoreboards; string comparisons use command storage.
