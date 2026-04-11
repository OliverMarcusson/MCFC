# Reset current
$execute as @e[type=minecraft:mannequin, tag=anpc, tag=npc] if score @s anpc.internal.npc.id matches $(id) run scoreboard players reset @s anpc.internal.npc.action_duration
$execute as @e[type=minecraft:wandering_trader, tag=anpc, tag=ai] if score @s anpc.internal.npc.id matches $(id) run data remove entity @s wander_target

$data modify storage anpc:internal npc.$(id).actions_copy set value [\
    { action: "pathfind", wander_target: [$(wander_target_x), $(wander_target_y), $(wander_target_z)] },\
    { action: "idle", duration: 100 },\
]
