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
void main() {
    Selector.of("@a").sendMessage("Coins pack loaded");
}
```

`main` runs every time the datapack loads, so it runs on world start and on every `/reload`. `Selector.of("@a")` matches every online player, and `sendMessage` sends them a chat message.

Run `/reload`. You should see `Coins pack loaded` in chat.

## 4. Store coins per player

```mcfc
@PlayerState("Coins")
int coins;

void main() {
    Selector.of("@a").sendMessage("Coins pack loaded");
}

@Every(ticks = 20)
void payday() {
    for (Player player : Selector.of("@a")) {
        player.state.coins = player.state.coins + 1;
        player.sendActionBar("Coins: $(player.state.coins)");
    }
}
```

- `@PlayerState("Coins") int coins;` declares a per-player integer. `"Coins"` is the display name of the scoreboard objective that backs it. A player who has never been paid reads as `0`.
- `@Every(ticks = 20)` runs `payday` every 20 ticks, which is once per second.
- `for (Player player : Selector.of("@a"))` runs the loop body once per online player, with `player` bound to that player.
- `$(...)` inside a string inserts a value.

Reload. The action bar now counts up once per second. Values are kept across reloads and restarts.

## 5. Move repeated logic into a function

The payout will be reused in step 7, so move it into a function:

```mcfc
@PlayerState("Coins")
int coins;

void main() {
    Selector.of("@a").sendMessage("Coins pack loaded");
}

void pay(Player player, int amount) {
    player.state.coins = player.state.coins + amount;
    player.sendActionBar("Coins: $(player.state.coins)");
}

@Every(ticks = 20)
void payday() {
    for (Player player : Selector.of("@a")) {
        pay(player, 1);
    }
}
```

The return type comes first (`void` if there isn't one), and every parameter has a type. `Player` is a single player. See [Types](/language/reference/types) for the rest.

## 6. React to events

Add these below `payday`:

```mcfc
@PlayerState("Coins")
int coins;

@EventHandler
void onPlayerJoin(PlayerJoinEvent event) {
    Player player = event.player();
    player.sendMessage("You earn 1 coin per second. Type /trigger buy to spend 10.");
}

@EventHandler
void onPlayerDeath(PlayerDeathEvent event) {
    Player player = event.player();
    var lost = player.state.coins / 2;
    player.state.coins = player.state.coins - lost;
    player.sendMessage("You dropped $(lost) coins.");
}
```

`@EventHandler` works like Bukkit: the parameter's type picks the event. `PlayerJoinEvent` runs once for each player, the first time the pack sees them. `PlayerDeathEvent` runs each time a player dies. `event.player` is that player. Integer `/` rounds down. The [event reference](/language/reference/events) lists every event.

To test, run `/kill` on yourself.

## 7. Add a command

```mcfc
@PlayerState("Coins")
int coins;

@Command("buy")
void buy(Player player) {
    if (player.state.coins < 10) {
        player.sendMessage("You need 10 coins.");
        return;
    }
    player.state.coins = player.state.coins - 10;
    player.give("minecraft:diamond", 1);
}
```

Players run this with `/trigger buy`. Vanilla has no custom commands, so `@Command` works through a trigger objective, which any player can run without operator permissions. The optional [agent](/runtime/mcfd-agent) also registers it as a real `/buy` command.

`return;` ends the handler early.

## 8. Wait without blocking

Minecraft commands can't pause. MCFC compiles `sleep` into a scheduled continuation, so the rest of the game keeps running while a function waits:

```mcfc
void remind(Player player) {
    async {
        sleep(3);
        player.sendMessage("Spend wisely.");
    }
}
```

Call `remind(player);` at the end of `buy`. `async { ... }` starts its body and returns immediately. `sleep(3)` waits 3 seconds inside that body, and `sleepTicks(n)` waits by ticks. Local variables such as `player` are copied when the block starts.

## The finished pack

```mcfc
@PlayerState("Coins")
int coins;

void main() {
    Selector.of("@a").sendMessage("Coins pack loaded");
}

void pay(Player player, int amount) {
    player.state.coins = player.state.coins + amount;
    player.sendActionBar("Coins: $(player.state.coins)");
}

@Every(ticks = 20)
void payday() {
    for (Player player : Selector.of("@a")) {
        pay(player, 1);
    }
}

@EventHandler
void onPlayerJoin(PlayerJoinEvent event) {
    Player player = event.player();
    player.sendMessage("You earn 1 coin per second. Type /trigger buy to spend 10.");
}

@EventHandler
void onPlayerDeath(PlayerDeathEvent event) {
    Player player = event.player();
    var lost = player.state.coins / 2;
    player.state.coins = player.state.coins - lost;
    player.sendMessage("You dropped $(lost) coins.");
}

@Command("buy")
void buy(Player player) {
    if (player.state.coins < 10) {
        player.sendMessage("You need 10 coins.");
        return;
    }
    player.state.coins = player.state.coins - 10;
    player.give("minecraft:diamond", 1);
    remind(player);
}

void remind(Player player) {
    async {
        sleep(3);
        player.sendMessage("Spend wisely.");
    }
}
```

## What got generated

Open the output folder. The files you'd call by hand are in `data/coins/function/`. Everything under `generated/` is internal. `main` is registered in the `minecraft:load` tag, and the task, events and command run from a generated tick function. See [How MCFC compiles](/language/reference/lowering) for the details.

## Next

- [Cookbook](./cookbook) has short recipes for common tasks.
- [Language Tour](/language/tour) covers the whole language on one page.
- [VS Code](/editor/vscode) adds diagnostics, completion and hover while you type.
