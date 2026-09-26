# mcfd

`mcfd` is the helper service behind [host calls](./host-bridge). It runs next to Minecraft, reads requests from the game log and writes answers back into the datapack. You only need it for packs that enable `[helper]` capabilities.

## Install

From a clone of the repository:

```powershell
cargo install --path mcfd
mcfd service install
```

`service install` registers `mcfd` to start at logon, as the `MCFC mcfd` scheduled task, and starts it. If creating the task is denied, it falls back to a per-user Run entry.

As an alternative, build a Windows installer that also bundles the agent. This needs Inno Setup 6:

```powershell
.\scripts\package-mcfd.ps1
```

## Check it's working

```powershell
mcfd service status
```

This lists the packs `mcfd` found. It looks for `mcfd.pack.toml` files, which `mcfc` generates, in the world `datapacks/` folders of known launchers. For other instance folders, set `MCFD_MINECRAFT_DIRS` to a `;`-separated list of paths.

In game, `mcfd.ping()` checks the whole round trip:

```mcfc
void health() {
    var r = mcfd.ping();
    if (r.ok) {
        debug("mcfd connected");
    } else {
        debug("mcfd not responding");
    }
}
```

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Every host call returns `ok = false` after a delay | `mcfd service status` shows the service running and lists the pack. |
| The pack isn't listed | `mcfd.pack.toml` exists in the deployed datapack. Rebuild after changing capabilities. For a custom launcher folder, set `MCFD_MINECRAFT_DIRS`. |
| HTTP is rejected | The domain is in `allow_domains`. |
| Requests never arrive | Look in `logs/latest.log` for `[mcfc_rpc]` lines. |

Per-pack secrets, such as a `bearer_token_env` value, can go in a `.env` file next to `mcfd.pack.toml`. Put it in the project's `assets/` folder so the build copies it there.

## Signing releases

`package-mcfd.ps1` produces unsigned builds with a `.sha256` checksum. To sign the executable and the installer, set `MCFD_SIGN_COMMAND` to a command that contains `{file}`.
