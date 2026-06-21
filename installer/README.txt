MCFC mcfd
==========

mcfd is the standalone host-bridge service for MCFC datapacks. The installer
requests administrator approval to create the "MCFC mcfd" Scheduled Task, then
starts it at user logon with limited privileges.

mcfd is installed globally under Program Files\MCFC\mcfd.

The service discovers generated mcfd.pack.toml descriptors in installed
datapacks and supports launcher-specific Minecraft instance locations.

Uninstalling mcfd removes the service and its local runtime data. It does not
remove your Minecraft worlds, datapacks, mcfd.pack.toml descriptors, or logs.
