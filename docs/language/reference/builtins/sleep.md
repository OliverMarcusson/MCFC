# `sleep`

```mcfc
sleep(seconds: int) -> void
```

Pauses the current execution path by seconds.

```mcfc
fn delayed() -> void:
    async:
        sleep(5)
        debug("five seconds later")
```

Inside `async`, the sleep pauses only that async branch.

A function that sleeps, sorts, or waits on a host call can pause. Calling it
pauses the caller too, and the caller carries on once the callee finishes:

```mcfc
fn wait_then_double(n: int) -> int:
    sleep_ticks(20)
    return n * 2

fn main() -> void:
    let x = wait_then_double(4)
    debug("one second later, x is $(x)")
```

A call to a function that can pause has to be a statement of its own:
`f()`, `let x = f()`, `x = f()`, or `return f()`. Inside a condition or
another expression, it is a compile error.

## Under The Hood

`sleep` splits the current lowered function at the call site. MCFC emits a continuation function for the remaining statements and schedules it with Minecraft `schedule function` after converting seconds to ticks.
