# Cookbook

Short, complete programs for common tasks. Each one compiles as-is in a project's `src/main.mcf`.

## Run code on a timer

```mcfc
fn tick() -> void:
    for player in selector("@a[y=-64,dy=0]"):
        player.teleport(block("0 100 0"))

task announce every_ticks(1200):
    selector("@a").tellraw("One minute has passed")
```

`fn tick()` runs every tick (20 times per second). A `task` with `every_ticks(n)` runs every `n` ticks. Keep `tick` small, since it runs 20 times a second.

## Choose where to store data

| You need | Use |
| --- | --- |
| A number or flag per player | `player_state name: int = "Display"` |
| Text, a decimal or a struct per player | `player_state name: string = "..."` (any declared type works) |
| A value per entity | `entity_state name: type` |
| A temporary value inside one function | `let` |
| A marker you check with selectors | `add_tag` / `has_tag`, or `@a[tag=...]` |

State persists across reloads and restarts. `int` and `bool` state is stored in scoreboards, so it also shows up in `/scoreboard` and works in selectors such as `@a[scores={mcfs_coins=10..}]`.

## Cooldowns

```mcfc
player_state last_dash: int = "Last dash"

command dash:
    let player = single(selector("@s"))
    let now = game_time()
    if now - player.state.last_dash < 100:
        player.actionbar("Dash is on cooldown")
        return
    player.state.last_dash = now
    player.effect("minecraft:speed", 2, 4)
```

`game_time()` counts ticks and never goes backwards, so it's safe to compare against a stored value.

## Countdowns and delays

```mcfc
fn start_round() -> void:
    async:
        for i in 0..5:
            selector("@a").title("$(5 - i)")
            sleep(1)
        selector("@a").title("Go!")
```

`sleep` pauses only the code inside `async:`. Without `async`, the function calling `start_round` would also wait.

## Give a custom item

```mcfc
fn give_blade(player: player_ref) -> void:
    let sword = item("minecraft:netherite_sword")
    sword.name = "Blade of Dawn"
    sword.nbt.CustomModelData = 7
    player.give(sword)
```

See [Builders](/language/reference/builders#item-builders) for the item fields.

## Spawn a custom mob

```mcfc
fn spawn_guard() -> void:
    let guard = entity("minecraft:iron_golem")
    guard.name = "Gate Guard"
    guard.name_visible = true
    guard.no_ai = true
    guard.tags = ["guard"]
    let spawned = block("0 64 0").summon(guard)
    spawned.state.post = "north gate"

entity_state post: string
```

## Show a bossbar timer

```mcfc
fn run_timer() -> void:
    let bar = bossbar("mypack:timer", "Time left")
    bar.max = 30
    bar.value = 30
    bar.players = selector("@a")
    bar.visible = true
    async:
        for i in 0..30:
            sleep(1)
            bar.value = 29 - i
        bar.remove()
```

## Clickable chat message

```mcfc
command accept:
    single(selector("@s")).tellraw("Accepted")

fn offer(player: player_ref) -> void:
    let msg = text("[Click to accept]")
    msg.color = "green"
    msg.click_event.action = "run_command"
    msg.click_event.command = "/trigger mcfcc_accept"
    player.tellraw(msg)
```

Clicking the message runs the `accept` command as that player.

## Split code across files

<!-- no-check -->
```mcfc
# src/main.mcf
mod shop

fn main() -> void:
    shop::setup()
```

<!-- no-check -->
```mcfc
# src/shop.mcf
pub fn setup() -> void:
    selector("@a").tellraw("Shop ready")
```

See [`mod`](/language/reference/statements#mod-and-pub) for where module files have to go.

## Call a web API

Enable the capability in `mcfc.toml`, and install `mcfd` with `mcfd service install`:

```toml
[helper]
backend = "mcfd"

[helper.capabilities]
http = { allow_domains = ["api.example.com"] }
```

```mcfc
command motd:
    let player = single(selector("@s"))
    greet(player)

fn greet(player: player_ref) -> void:
    let r = http.get("https://api.example.com/motd")
    if r.ok:
        player.tellraw(r.body)
    else:
        player.tellraw("No message today")
```

If `mcfd` isn't running, the call times out and `r.ok` is `false`. See [Capabilities](/runtime/capabilities).

## Cancel chat or block breaking

This needs the [agent](/runtime/mcfd-agent) (`[helper.agent] enabled = true`):

```mcfc
event chat(event: chat_event):
    if event.message == "spoiler":
        event.cancel()

event block_break(event: block_break_event):
    if event.player.has_tag("spawn_protected"):
        event.cancel()
        event.player.actionbar("Spawn is protected")
```

## Debug a value

```mcfc
fn main() -> void:
    let player = single(selector("@p"))
    debug("health=$(player.health()) food=$(player.food())")
```

`debug` sends the message to every player's chat. To see the generated commands, build with `--no-optimize` and open `dist/data/<namespace>/function/generated/`.
