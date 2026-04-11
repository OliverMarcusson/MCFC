scoreboard players set #interaction_cooldown anpc.api.npc 10

function anpc:api/reset

data modify storage anpc:api npc set value {\
    profile: { name: "7ks" },\
    custom_name: {text: "Example NPC", color: "white"},\
    movement_speed: 0.6f,\
    no_ai: false,\
    pose: "standing",\
    rotation: [0.0f, 0.0f],\
    actions: [\
        { action: "pathfind", wander_target: [I; 0, 0, 0] },\
        { action: "jump" },\
        { action: "set_profile", profile: { name: "7ks" } },\
        { action: "set_movement_speed", movement_speed: 1.0f },\
        { action: "set_ai", no_ai: true },\
        { action: "pose", pose: "crouching" },\
        { action: "rotate", rotation: [0.0f, 0.0f] },\
        { action: "idle", duration: 10 },\
        { action: "run_function", function: "foo:bar" },\
        { action: "position", position: [0.0f, 0.0f, 0.0f] },\
        { action: "idle", duration: -1 },\
    ],\
    dialogues: [\
        [\
            { text: { text: "Placeholder 1", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 2", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 3", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
        ],\
        [\
            { text: { text: "Placeholder 4", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 5", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 6", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
        ],\
    ],\
}


data modify storage anpc:api npc set value {\
    profile: { name: "7ks" },\
    custom_name: {text: "Example NPC", color: "white"},\
    movement_speed: 0.6f,\
    no_ai: false,\
    pose: "standing",\
    rotation: [0.0f, 0.0f],\
    actions: [\
        {action: "pathfind", wander_target: [0, 56, 0]},\
        {action: "pathfind", wander_target: [0, 56, 10]},\
    ],\
    dialogues: [\
        [\
            { text: { text: "Placeholder 1", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 2", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 3", color: "white" }, duration: -1, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
        ],\
        [\
            { text: { text: "Placeholder 4", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 5", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
            { text: { text: "Placeholder 6", color: "white" }, duration: 100, sound: { id: "minecraft:entity.villager.ambient", volume: 1.0f, pitch: 1.0f } },\
        ],\
    ],\
    guard_vision: 6,\
}


function anpc:api/create_guard