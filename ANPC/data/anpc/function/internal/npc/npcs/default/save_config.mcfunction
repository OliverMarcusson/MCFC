$data modify storage anpc:internal npc.$(id).id set from storage anpc:api npc.id
$data modify storage anpc:internal npc.$(id).movement_speed set from storage anpc:api npc.movement_speed
$execute if data storage anpc:api npc.actions[0] run data modify storage anpc:internal npc.$(id).actions set from storage anpc:api npc.actions
