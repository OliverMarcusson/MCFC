tag @s add current

execute as @e[tag=anpc, tag=component, tag=hitbox] if function anpc:internal/npc/component/hitbox/is_interacted_with at @s run function anpc:internal/npc/component/hitbox/on_interact

tag @s remove current

advancement revoke @s only anpc:internal/interacted_with_interaction
advancement revoke @s only anpc:internal/hurt_interaction
