# Advanced NPC

A Minecraft datapack for configurable NPCs with scripted actions, dialogues, and more.

## Features
- **Actions** - Action sequences (move, jump, pose, rotate, etc.)
- **Dialogues** - Manual or automatic dialogues with custom sound support

## How to use

1. Place the datapack into your world's `datapacks/` directory
2. Run `/reload`
3. Configure in storage
3. Run `/function anpc:api/create`

## Configuration

NPCs are configured through the `anpc` storage. Here's a complete example:

```mcfunction
data modify storage anpc:api npc set value {\
    profile: { name: "7ks" },\
    custom_name: {text: "Example NPC", color: "white"},\
    movement_speed: 0.6f,\
    no_ai: false,\
    pose: "standing",\
    rotation: [0.0f, 0.0f],\
    actions: [\
        { type: "move", wander_target: [I; 0, 0, 0] },\
        { type: "jump" },\
        { type: "set_profile", profile: { name: "7ks" } },\
        { type: "set_movement_speed", movement_speed: 1.0f },\
        { type: "set_ai", no_ai: true },\
        { type: "pose", pose: "crouching" },\
        { type: "rotate", rotation: [0.0f, 0.0f] },\
        { type: "idle", duration: 10 },\
        { type: "run_function", function: "foo:bar" },\
        { type: "idle", duration: -1 },\
    ],\
    dialogues: [\
        [\
            { text: { text: "Placeholder 1", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.yes", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 2", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.yes", volume: 1.0f, pitch: 1.0f }},\
            { text: { text: "Placeholder 3", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.yes", volume: 1.0f, pitch: 1.0f }},\
        ],\
        [\
            { text: { text: "Placeholder 4", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.yes", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 5", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.yes", volume: 1.0f, pitch: 1.0f }},\
            { text: { text: "Placeholder 6", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.yes", volume: 1.0f, pitch: 1.0f }},\
        ],\
    ],\
}
```
