# `task`

Declares a scheduled Bukkit-style task.

```mcfc
task heartbeat every_ticks(20):
    debug("heartbeat")

task setup after_ticks(1):
    debug("setup")
```

`every_ticks(n)` repeats. `after_ticks(n)` runs once after datapack load.

