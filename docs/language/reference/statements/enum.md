# `enum`

Declares a named type with a fixed set of constants. An enum is a top-level declaration. Write one constant per indented line; an enum must contain at least one constant.

```mcfc
enum Mode:
    SURVIVAL
    CREATIVE

fn describe(mode: Mode) -> void:
    switch mode:
        case Mode.SURVIVAL:
            debug("Survival")
        case Mode.CREATIVE:
            debug("Creative")
```

Use `Mode.SURVIVAL` to refer to a constant. Enum constants are checked against their declared enum type, and duplicate constant names are rejected.

## Under The Hood

Enum constants are assigned integer values in declaration order, starting at zero. Enum values live in scoreboard slots, so reordering constants changes their generated numeric values.
