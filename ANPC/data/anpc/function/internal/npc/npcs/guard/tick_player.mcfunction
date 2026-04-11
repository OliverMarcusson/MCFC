tag @s add current

$execute anchored eyes positioned ^ ^ ^ as @e[type=minecraft:mannequin, tag=anpc, tag=npc, tag=current, distance=..$(guard_vision), limit=1] facing entity @s eyes positioned as @s positioned ^ ^ ^1 rotated as @s positioned ^ ^ ^1 if entity @s[distance=..0.7] run scoreboard players operation @s anpc.internal.npc.found_player = @a[tag=current, limit=1] anpc.internal.player.id

tag @s remove current
