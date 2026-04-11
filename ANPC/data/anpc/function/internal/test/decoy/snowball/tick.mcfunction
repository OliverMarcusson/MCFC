execute unless entity @s[tag=has_rider] run summon minecraft:marker ~ ~ ~ {Tags: ["anpc", "decoy"]}
ride @n[type=minecraft:marker, tag=anpc, tag=decoy] mount @s
tag @s add has_rider
