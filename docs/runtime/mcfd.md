# mcfd

`mcfd` is the optional host-bridge service for MCFC datapacks that use helper capabilities. It is a standalone Rust binary and does not require a mod loader.

## Build

```powershell
cargo build -p mcfd --release
```

## Service Commands

Install or start the helper:

```powershell
mcfd service install
```

Check discovered packs and helper state:

```powershell
mcfd service status
mcfd agent status
```

The Windows installer creates the `MCFC mcfd` user-logon Scheduled Task, then starts it with limited privileges. If task creation is denied, it falls back to a per-user Windows Run entry.

## Discovery

`mcfd` searches known Minecraft launcher locations for generated `mcfd.pack.toml` descriptors under world `datapacks/` directories. Custom instance roots can be added with the `MCFD_MINECRAFT_DIRS` environment variable as a semicolon-separated list.

## Packaging

Create the x64 installer with:

```powershell
.\scripts\package-mcfd.ps1
```

Inno Setup 6 is required. Releases are unsigned by default and include a `.sha256` checksum. Set `MCFD_SIGN_COMMAND` to a trusted command template containing `{file}` to sign the executable and installer during packaging.

## Troubleshooting

- Rebuild and redeploy the datapack after manifest capability changes.
- Confirm the generated `mcfd.pack.toml` exists in the deployed datapack.
- Check `mcfd service status` for discovered packs.
- Check Minecraft `logs/latest.log` if requests appear to time out.
- Use `.env` beside the descriptor for per-pack secrets when an example expects it.
