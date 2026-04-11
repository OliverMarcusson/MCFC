execute unless entity @s[tag=disabled_movement] run return fail
execute if score @e[tag=anpc, tag=component, tag=dialogue_box, tag=current, limit=1] anpc.internal.npc.dialogue_duration matches -1 run return fail
execute if score @e[tag=anpc, tag=component, tag=dialogue_box, tag=current, limit=1] anpc.internal.npc.dialogue_duration matches 0.. run return fail
execute if entity @e[tag=anpc, tag=npc, tag=guard, tag=current, tag=alerted, limit=1] run return fail

return 1
