# The Oracle of Numbers

A fun showcase of the mcfc **host bridge**: a vanilla datapack that talks to the
real world through the `mcfd` helper.

What it does:

- **Every 15 seconds** the Oracle fetches a live random number fact from
  `numbersapi.com` (`http`), reads the real-world clock (`time`), and proclaims the
  fact to everyone online — stamped with the timestamp and a per-player
  "prophecies witnessed" counter (`player.state`, shown on the scoreboard).
- **`/function oracle:roll`** rolls a die using the helper's randomness (`rand`);
  roll a 6 for a little celebration.

Capabilities used: `http` (allow-listed to `numbersapi.com`), `time`, `rand`.

## Build

```powershell
cargo build -p mcfd --release
cargo run --bin mcfc -- build examples/oracle/mcfc.toml --out examples/oracle/dist
```

This writes the datapack to `dist/`, emits `dist/mcfd.toml`, and copies the `mcfd`
helper next to it.

## Run

1. Copy `dist/` into your world's `datapacks/` folder.
2. From the `dist/` folder, start the helper: `./mcfd mcfd.toml` (it auto-detects
   `logs/latest.log` from the datapack location — no path to configure).
3. Load the world (or `/reload`). The Oracle awakens and starts prophesying; try
   `/function oracle:roll` too.

Each request prints a single `[mcfc_rpc]` marker line in chat — that's the one
visible part of the single-player transport. If `mcfd` isn't running, calls fall
back to a timeout and the Oracle says it's "silent".

See [`src/main.mcf`](src/main.mcf) for the (short!) source.
