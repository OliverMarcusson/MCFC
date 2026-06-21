data modify storage debug_function_log:probe req set value {mcpipe:1b,protocol:2,pack:"debug_function_log",id:7,mod:"mcfd",fn:"ping",args:[]}
summon minecraft:pig ~ ~1000 ~ {Tags:["debug_function_log_probe"],Age:-24000,Health:1f,NoAI:1b,Silent:1b}
function debug_function_log:emit_name with storage debug_function_log:probe
damage @e[type=minecraft:pig,tag=debug_function_log_probe,sort=nearest,limit=1] 1 minecraft:generic_kill
