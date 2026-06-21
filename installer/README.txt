MCFC mcfd
==========

mcfd is the standalone host-bridge service for MCFC datapacks. The installer
creates the "MCFC mcfd" Scheduled Task when Windows permits it. If task
creation is denied for the current user, it registers a per-user Windows Run
entry instead; either mechanism starts the service at user logon.

mcfd is installed globally under Program Files\MCFC\mcfd.

The service discovers generated mcfd.pack.toml descriptors in installed
datapacks and supports launcher-specific Minecraft instance locations.

Uninstalling mcfd removes the service and its local runtime data. It does not
remove your Minecraft worlds, datapacks, mcfd.pack.toml descriptors, or logs.
