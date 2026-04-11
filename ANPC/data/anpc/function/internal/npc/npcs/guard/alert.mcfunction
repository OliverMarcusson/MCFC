execute on passengers if entity @s[tag=guard_tooltip] run function anpc:internal/npc/component/guard_tooltip/show
execute on passengers if entity @s[tag=guard_vision] run data modify entity @s Glowing set value true

playsound minecraft:entity.villager.no master @a ~ ~ ~ 1.0 1.0

tag @s add alerted
