execute if entity @s[tag=ai] run function anpc:internal/npc/component/ai/tick with storage anpc:internal npc
execute if entity @s[tag=hitbox] run function anpc:internal/npc/component/hitbox/tick
execute if entity @s[tag=custom_name_tooltip] run function anpc:internal/npc/component/custom_name_tooltip/tick
execute if entity @s[tag=dialogue_box] run function anpc:internal/npc/component/dialogue_box/tick with storage anpc:internal npc
execute if entity @s[tag=dialogue_tooltip] run function anpc:internal/npc/component/dialogue_tooltip/tick
execute if entity @s[tag=guard_vision] run function anpc:internal/npc/component/guard_vision/tick with storage anpc:internal npc
