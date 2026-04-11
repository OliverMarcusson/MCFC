$data modify entity @s pose set from storage anpc:internal npc.$(id).actions_copy[0].pose

data modify storage anpc:internal npc.pose set value "standing"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.pose set from storage anpc:internal npc.$(id).actions_copy[0].pose
execute if score #success anpc.internal matches 0 on passengers run function anpc:internal/npc/component/on_standing

data modify storage anpc:internal npc.pose set value "crouching"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.pose set from storage anpc:internal npc.$(id).actions_copy[0].pose
execute if score #success anpc.internal matches 0 on passengers run function anpc:internal/npc/component/on_crouching

data modify storage anpc:internal npc.pose set value "swimming"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.pose set from storage anpc:internal npc.$(id).actions_copy[0].pose
execute if score #success anpc.internal matches 0 on passengers run function anpc:internal/npc/component/on_swimming

data modify storage anpc:internal npc.pose set value "sleeping"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.pose set from storage anpc:internal npc.$(id).actions_copy[0].pose
execute if score #success anpc.internal matches 0 on passengers run function anpc:internal/npc/component/on_sleeping
