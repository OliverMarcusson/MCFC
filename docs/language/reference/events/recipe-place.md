# `recipe_place`

```mcfc
event recipe_place(event: recipe_place_event):
```

Agent-backed recipe placement event.

Fields: `player`, `container_id`, `recipe`, `use_max_items`, `cancelled`. Cancellation: yes.

```mcfc
event recipe_place(event: recipe_place_event):
    event.player.actionbar("recipe $(event.recipe)")
```

