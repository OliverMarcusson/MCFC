execute as @e[type=minecraft:snowball] at @s run function anpc:internal/test/decoy/snowball/tick
execute as @e[type=minecraft:marker, tag=anpc, tag=decoy] unless predicate anpc:has_vehicle at @s align xz positioned ~0.5 ~ ~0.5 run function anpc:internal/test/decoy/activate

