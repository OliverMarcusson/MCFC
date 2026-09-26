# Your First Pack

This tutorial builds a small economy pack. Players earn one coin per second and lose half their coins when they die. They can trade 10 coins for a diamond with a command. Each step adds one language feature and ends with a build you can test in game.

You need `mcfc` installed ([Getting Started](./getting-started)) and a Minecraft 26.3 world with cheats enabled.

## 1. Create the project

```powershell
mcfc new coins --helper none
cd coins
```

This creates the project:

```text
coins/
  mcfc.toml       # namespace, source and output directories
  src/main.mcf    # the root source file
  assets/         # files copied into the datapack as-is
```

`mcfc.toml` sets the namespace to `coins` and the output directory to `dist`.

## 2. Build straight into your world

Have the compiler write straight into the world's datapack folder, so you don't copy files after each build. The world lives in `.minecraft/saves/<world>/`. On Windows that's usually `%APPDATA%\.minecraft\saves\<world>\`.

```powershell
mcfc watch . --out "$env:APPDATA\.minecraft\saves\<world>\datapacks\coins"
```

`watch` builds once, then rebuilds each time you save a `.mcf` file. Leave it running. After each change, run `/reload` in game.

## 3. Run code on load

Replace `src/main.mcf` with:

```mcfc
fn main() -> void:
    selector("@a").tellraw("Coins pack loaded")
```

`main` runs every time the datapack loads, so it runs on world start and on every `/reload`. `selector("@a")` matches every online player, and `tellraw` sends them a chat message.

Run `/reload`. You should see `Coins pack loaded` in chat.

## 4. Store coins per player

```mcfc
player_state coins: int = "Coins"

fn main() -> void:
    selector("@a").tellraw("Coins pack loaded")

task payday every_ticks(20):
    for player in selector("@a"):
        player.state.coins = player.state.coins + 1
        player.actionbar("Coins: $(player.state.coins)")
```

- `player_state coins: int = "Coins"` declares a per-player integer. `"Coins"` is the display name of the scoreboard objective that backs it. A player who has never been paid reads as `0`.
- `task payday every_ticks(20):` runs the body every 20 ticks, which is once per second.
- `for player in selector("@a"):` runs the loop body once per online player, with `player` bound to that player.
- `$(...)` inside a string inserts a value.

Reload. The action bar now counts up once per second. Values are kept across reloads and restarts.

## 5. Move repeated logic into a function

The payout will be reused in step 7, so move it into a function:

```mcfc
player_state coins: int = "Coins"

fn main() -> void:
    selector("@a").tellraw("Coins pack loaded")

fn pay(player: player_ref, amount: int) -> void:
    player.state.coins = player.state.coins + amount
    player.actionbar("Coins: $(player.state.coins)")

task payday every_ticks(20):
    for player in selector("@a"):
        pay(player, 1)
```

Every parameter needs a type, and so does the return value (`void` if there isn't one). `player_ref` is a single player. See [Types](/language/reference/types) for the rest.

## 6. React to events

Add these below `payday`:

```mcfc
player_state coins: int = "Coins"

event player_join:
    let player = single(selector("@s"))
    player.tellraw("You earn 1 coin per second. Type /trigger mcfcc_buy to spend 10.")

event player_death:
    let player = single(selector("@s"))
    let lost = player.state.coins / 2
    player.state.coins = player.state.coins - lost
    player.tellraw("You dropped $(lost) coins.")
```

Event handlers run as the affected player, so `@s` is that player. `selector(...)` can match any number of entities. `single(...)` narrows the selection to one player so you can use player methods.

`player_join` runs once for each player, the first time the pack sees them. `player_death` runs each time a player dies. Integer `/` rounds down. The [event reference](/language/reference/events) lists every event.

To test, run `/kill` on yourself.

## 7. Add a command

```mcfc
player_state coins: int = "Coins"

command buy:
    let player = single(selector("@s"))
    if player.state.coins < 10:
        player.tellraw("You need 10 coins.")
        return
    player.state.coins = player.state.coins - 10
    player.give("minecraft:diamond", 1)
```

Players run this with `/trigger mcfcc_buy`. Vanilla has no custom commands, so `command` works through a trigger objective, which any player can run without operator permissions. The optional [agent](/runtime/mcfd-agent) also registers it as a real `/buy` command.

`return` ends the handler early.

## 8. Wait without blocking

Minecraft commands can't pause. MCFC compiles `sleep` into a scheduled continuation, so the rest of the game keeps running while a function waits:

```mcfc
fn remind(player: player_ref) -> void:
    async:
        sleep(3)
        player.tellraw("Spend wisely.")
```

Call `remind(player)` at the end of `buy`. `async:` starts its body and returns immediately. `sleep(3)` waits 3 seconds inside that body, and `sleep_ticks(n)` waits by ticks. Local variables such as `player` are copied when the block starts.

## The finished pack

```mcfc
player_state coins: int = "Coins"

fn main() -> void:
    selector("@a").tellraw("Coins pack loaded")

fn pay(player: player_ref, amount: int) -> void:
    player.state.coins = player.state.coins + amount
    player.actionbar("Coins: $(player.state.coins)")

task payday every_ticks(20):
    for player in selector("@a"):
        pay(player, 1)

event player_join:
    let player = single(selector("@s"))
    player.tellraw("You earn 1 coin per second. Type /trigger mcfcc_buy to spend 10.")

event player_death:
    let player = single(selector("@s"))
    let lost = player.state.coins / 2
    player.state.coins = player.state.coins - lost
    player.tellraw("You dropped $(lost) coins.")

command buy:
    let player = single(selector("@s"))
    if player.state.coins < 10:
        player.tellraw("You need 10 coins.")
        return
    player.state.coins = player.state.coins - 10
    player.give("minecraft:diamond", 1)
    remind(player)

fn remind(player: player_ref) -> void:
    async:
        sleep(3)
        player.tellraw("Spend wisely.")
```

## What got generated

Open the output folder. The files you'd call by hand are in `data/coins/function/`. Everything under `generated/` is internal. `main` is registered in the `minecraft:load` tag, and the task, events and command run from a generated tick function. See [How MCFC compiles](/language/reference/lowering) for the details.

## Next

- [Cookbook](./cookbook) has short recipes for common tasks.
- [Language Tour](/language/tour) covers the whole language on one page.
- [VS Code](/editor/vscode) adds diagnostics, completion and hover while you type.
