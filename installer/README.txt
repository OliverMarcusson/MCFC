MCFC mcfd
==========

mcfd is the standalone host-bridge service for MCFC datapacks. The installer
creates the "MCFC mcfd" Scheduled Task when Windows permits it. If task
creation is denied for the current user, it registers a per-user Windows Run
entry instead; either mechanism starts the service at user logon.

mcfd is installed globally under Program Files\MCFC\mcfd.

The service discovers generated mcfd.pack.toml descriptors in installed
datapacks and supports launcher-specific Minecraft instance locations.

The installation also contains the optional `mcfd-agent.jar` and its Attach API
launcher. When a discovered pack requests `[helper.agent] enabled = true`, the
service automatically makes one best-effort attachment attempt to the matching
Minecraft JVM. Dynamic agent attachment is best-effort and never affects ordinary
vanilla datapacks.

Uninstalling mcfd removes the service and its local runtime data. It does not
remove your Minecraft worlds, datapacks, mcfd.pack.toml descriptors, or logs.
