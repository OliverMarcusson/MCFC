execute on passengers if entity @s[tag=guard_tooltip] run function anpc:internal/npc/component/guard_tooltip/hide
execute on passengers if entity @s[tag=guard_vision] run data modify entity @s Glowing set value false

scoreboard players reset @s anpc.internal.npc.found_player

tag @s remove alerted
