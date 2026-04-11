# Check npc data
execute unless data storage anpc:api npc.profile run data modify storage anpc:api npc.profile set value {}
execute unless data storage anpc:api npc.custom_name run data modify storage anpc:api npc.custom_name set value {}
execute unless data storage anpc:api npc.movement_speed run data modify storage anpc:api npc.movement_speed set value 0.0f
execute unless data storage anpc:api npc.no_ai run data modify storage anpc:api npc.no_ai set value true
execute unless data storage anpc:api npc.pose run data modify storage anpc:api npc.pose set value "standing"
execute unless data storage anpc:api npc.actions run data modify storage anpc:api npc.actions set value []
execute unless data storage anpc:api npc.guard_vision run data modify storage anpc:api npc.guard_vision set value 10.0f

# Store ID
execute store result storage anpc:api npc.id int 1 run scoreboard players get #next anpc.internal.npc.id

# Save npc data
function anpc:internal/npc/npcs/guard/save_config with storage anpc:api npc

# Spawn npc
function anpc:internal/npc/npcs/guard/spawn with storage anpc:api npc

# On spawn
function anpc:internal/on_spawn_npc

# Remove ID
data remove storage anpc:api npc.id
