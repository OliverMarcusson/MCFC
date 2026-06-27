# Generated Project

`mcfc new` creates a small project that is ready to build.

```powershell
mcfc new my-pack --helper none
```

Generated layout:

```text
my-pack/
  mcfc.toml
  README.md
  .gitignore
  assets/
    .gitkeep
  src/
    main.mcf
```

Plain projects start with a simple `main` function:

```mcfc
fn main() -> void:
    let player = single(selector("@p"))
    if exists(player):
        player.tellraw("MCFC is live.")
```

Helper projects scaffold manifest capability settings and starter code for `time.now()` and `rand.int(...)`. Agent projects also include a `command status:` declaration and a typed `event chat(...)` example.
