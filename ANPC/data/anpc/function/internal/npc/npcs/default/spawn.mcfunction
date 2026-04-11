$summon minecraft:wandering_trader ~ ~ ~ {\
    Team: "anpc",\
    Tags: ["anpc", "component", "ai", "default"],\
    Silent: true,\
    Invulnerable: true,\
    Offers: { Recipes: [] },\
    NoAI: $(no_ai),\
    Rotation: $(rotation),\
    attributes: [\
        {\
            id: "minecraft:scale",\
            base: 0.8,\
        },\
        {\
            id: "minecraft:movement_speed",\
            base: $(movement_speed),\
        },\
    ],\
    active_effects: [\
        {\
            id: "minecraft:invisibility",\
            duration: -1,\
            show_particles: false,\
        },\
    ],\
}

execute store result score #result anpc.internal run data get storage anpc:api npc.custom_name

$execute unless score #result anpc.internal matches 0 run summon minecraft:mannequin ~ ~ ~ {\
    Tags: ["anpc", "npc", "default"],\
    Team: "anpc",\
    profile: $(profile),\
    pose: $(pose),\
    immovable: true,\
    Silent: true,\
    Invulnerable: true,\
    Passengers: [\
        {\
            id: "minecraft:interaction",\
            Tags: ["anpc", "component", "hitbox", "lower", "default"],\
            height: -1.82d,\
            width: 0.6001d,\
            response: false,\
        },\
        {\
            id: "minecraft:interaction",\
            Tags: ["anpc", "component", "hitbox", "upper", "default"],\
            height: 0.02,\
            width: 0.6001d,\
            response: false,\
        },\
        {\
            id: "minecraft:text_display",\
            Tags: ["anpc", "component", "custom_name_tooltip", "default"],\
            text: $(custom_name),\
            billboard: "center",\
            teleport_duration: 1,\
            transformation: {\
                left_rotation: [0.0f, 0.0f, 0.0f, 1.0f],\
                right_rotation: [0.0f, 0.0f, 0.0f, 1.0f],\
                translation: [0.0f, 0.2f, 0.0f],\
                scale: [0.0f, 0.0f, 0.0f],\
            }\
        },\
    ],\
}

$execute if score #result anpc.internal matches 0 run summon minecraft:mannequin ~ ~ ~ {\
    Tags: ["anpc", "npc", "default"],\
    profile: $(profile),\
    pose: $(pose),\
    immovable: true,\
    Silent: true,\
    Invulnerable: true,\
    Passengers: [\
        {\
            id: "minecraft:interaction",\
            Tags: ["anpc", "component", "hitbox", "lower", "default"],\
            height: -1.82d,\
            width: 0.6001d,\
            response: false,\
        },\
        {\
            id: "minecraft:interaction",\
            Tags: ["anpc", "component", "hitbox", "upper", "default"],\
            height: 0.02,\
            width: 0.6001d,\
            response: false,\
        },\
    ],\
}
