execute if score @s anpc.internal.npc.dialogue_duration matches -1.. on vehicle at @s unless entity @a[tag=current, distance=..15] on passengers if entity @s[tag=dialogue_box] run function anpc:internal/npc/component/dialogue_box/end with storage anpc:internal npc

execute if score @s anpc.internal.npc.dialogue_duration matches 0 run function anpc:internal/npc/component/dialogue_box/continue with storage anpc:internal npc
execute if score @s anpc.internal.npc.dialogue_duration matches 1.. run scoreboard players remove @s anpc.internal.npc.dialogue_duration 1
