particle note ~ ~ ~ 0 0 0 0 3

playsound block.note_block.pling master @a ~ ~ ~ 1.0 1.0

tp @s ~ ~ ~

# CHECK REGIONS!!!

execute store result storage anpc:internal npc.id int 1 run scoreboard players get @n[type=minecraft:mannequin, tag=anpc, tag=npc] anpc.internal.npc.id
execute store result storage anpc:internal npc.wander_target_x int 1 run data get entity @s Pos[0]
execute store result storage anpc:internal npc.wander_target_y int 1 run data get entity @s Pos[1]
execute store result storage anpc:internal npc.wander_target_z int 1 run data get entity @s Pos[2]

function anpc:internal/test/decoy/set_target with storage anpc:internal npc

kill @s
