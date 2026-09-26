---
layout: home

hero:
  name: MCFC
  text: A typed language that compiles to Minecraft datapacks
  tagline: Write .mcf source with functions, types, loops and per-player state. Get a vanilla datapack for Minecraft 26.3.
  image:
    src: /MCFC-icon.png
    alt: MCFC icon
  actions:
    - theme: brand
      text: Your First Pack
      link: /guide/first-pack
    - theme: alt
      text: Install
      link: /guide/getting-started
    - theme: alt
      text: Reference
      link: /language/reference/statements

features:
  - title: Vanilla output
    details: The compiler writes plain .mcfunction files, tags and pack.mcmeta. No mod or plugin is needed to run the pack.
  - title: Checked before you load it
    details: Type errors, unknown methods and wrong arguments are reported with file and line, in the terminal and in VS Code.
  - title: Waiting without blocking
    details: sleep() and async blocks compile into scheduled functions, so a countdown is a for loop.
  - title: Optional host access
    details: With the mcfd helper, a pack can call HTTP APIs, read files, use SQLite, and get real time, each enabled per project.
---

```mcfc
@PlayerState("Coins")
int coins;

@Every(ticks = 20)
void payday() {
    for (Player player : Selector.of("@a")) {
        player.state.coins = player.state.coins + 1;
        player.sendActionBar("Coins: $(player.state.coins)");
    }
}

@Command("buy")
void buy(Player player) {
    if (player.state.coins >= 10) {
        player.state.coins = player.state.coins - 10;
        player.give("minecraft:diamond", 1);
    }
}
```

MCFC is early-stage. Syntax and output change between commits, so pin a commit for any pack you depend on.
