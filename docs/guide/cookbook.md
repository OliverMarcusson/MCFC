# Cookbook

Short, complete programs for common tasks. Each one compiles as-is in a project's `src/main.mcf`.

## Run code on a timer

```mcfc
void tick() {
    for (Player player : Selector.of("@a[y=-64,dy=0]")) {
        player.teleport(Block.of("0 100 0"));
    }
}

@Every(ticks = 1200)
void announce() {
    Selector.of("@a").sendMessage("One minute has passed");
}
```

`void tick()` runs every tick (20 times per second). A function with `@Every(ticks = n)` runs every `n` ticks. Keep `tick` small, since it runs 20 times a second.

## Choose where to store data

| You need | Use |
| --- | --- |
| A number or flag per player | `@PlayerState("Display") int name;` |
| Text, a decimal or a record per player | `@PlayerState String name;` (any declared type works) |
| A value per entity | `@EntityState Type name;` |
| A temporary value inside one function | `var` |
| A marker you check with selectors | `addTag` / `hasTag`, or `@a[tag=...]` |

State persists across reloads and restarts. `int` and `boolean` state is stored in scoreboards, so it also shows up in `/scoreboard` and works in selectors such as `@a[scores={mcfs_coins=10..}]`.

## Cooldowns

```mcfc
@PlayerState("Last dash")
int last_dash;

@Command("dash")
void dash(Player player) {
    var now = gameTime();
    if (now - player.state.last_dash < 100) {
        player.sendActionBar("Dash is on cooldown");
        return;
    }
    player.state.last_dash = now;
    player.effect("minecraft:speed", 2, 4);
}
```

`gameTime()` counts ticks and never goes backwards, so it's safe to compare against a stored value.

## Countdowns and delays

```mcfc
void startRound() {
    async {
        for (int i = 0; i < 5; i++) {
            Selector.of("@a").sendTitle("$(5 - i)");
            sleep(1);
        }
        Selector.of("@a").sendTitle("Go!");
    }
}
```

`sleep` pauses only the code inside `async`. Without `async`, the function calling `startRound` would also wait.

## Give a custom item

```mcfc
void giveBlade(Player player) {
    var sword = new ItemStack("minecraft:netherite_sword");
    sword.setName("Blade of Dawn");
    sword.nbt.CustomModelData = 7;
    player.give(sword);
}
```

See [Builders](/language/reference/builders#item-builders) for the item fields.

## Spawn a custom mob

```mcfc
void spawnGuard() {
    var guard = new EntityData("minecraft:iron_golem");
    guard.setName("Gate Guard");
    guard.setNameVisible(true);
    guard.setNoAi(true);
    guard.setTags(List.of("guard"));
    var spawned = Block.of("0 64 0").summon(guard);
    spawned.state.post = "north gate";
}

@EntityState
String post;
```

## Show a bossbar timer

```mcfc
void runTimer() {
    var bar = new BossBar("mypack:timer", "Time left");
    bar.setMax(30);
    bar.setValue(30);
    bar.setPlayers(Selector.of("@a"));
    bar.setVisible(true);
    async {
        for (int i = 0; i < 30; i++) {
            sleep(1);
            bar.setValue(29 - i);
        }
        bar.remove();
    }
}
```

## Clickable chat message

```mcfc
@Command("accept")
void accept(Player player) {
    player.sendMessage("Accepted");
}

void offer(Player player) {
    var msg = new Component("[Click to accept]");
    msg.color = "green";
    msg.click_event.action = "run_command";
    msg.click_event.command = "/trigger accept";
    player.sendMessage(msg);
}
```

Clicking the message runs the `accept` command as that player.

## Split code across files

<!-- no-check -->
```mcfc
// src/main.mcf

void main() {
    shop.setup();
}
```

<!-- no-check -->
```mcfc
// src/shop.mcf
public void setup() {
    Selector.of("@a").sendMessage("Shop ready");
}
```

Every `.mcf` file under `src/` is a module named by its path. See [Modules](/language/reference/statements#modules-and-public).

## Call a web API <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

Enable the capability in `mcfc.toml`, and install `mcfd` with `mcfd service install`:

```toml
[helper]
backend = "mcfd"

[helper.capabilities]
http = { allow_domains = ["api.example.com"] }
```

```mcfc
@Command("motd")
void motd(Player player) {
    greet(player);
}

void greet(Player player) {
    var r = http.get("https://api.example.com/motd");
    if (r.ok()) {
        player.sendMessage(r.body());
    } else {
        player.sendMessage("No message today");
    }
}
```

If `mcfd` isn't running, the call times out and `r.ok()` is `false`. See [Capabilities](/runtime/capabilities).

## Cancel chat or block breaking <Badge type="danger" text="Agent" title="Needs mcfd-agent running beside the server. Not available on Realms." />

This needs the [agent](/runtime/mcfd-agent) (`[helper.agent] enabled = true`):

```mcfc
@EventHandler
void onChat(ChatEvent event) {
    if (event.message() == "spoiler") {
        event.cancel();
    }
}

@EventHandler
void onBlockBreak(BlockBreakEvent event) {
    if (event.player().hasTag("spawn_protected")) {
        event.cancel();
        event.player().sendActionBar("Spawn is protected");
    }
}
```

## Debug a value

```mcfc
void main() {
    var player = Selector.of("@p").getFirst();
    debug("health=$(player.getHealth()) food=$(player.getFoodLevel())");
}
```

`debug` sends the message to every player's chat. To see the generated commands, build with `--no-optimize` and open `dist/data/<namespace>/function/generated/`.
