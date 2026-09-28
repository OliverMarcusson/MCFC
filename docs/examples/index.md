# Examples

Complete project in [`examples/`](https://github.com/OliverMarcusson/MCFC/tree/main/examples). Build it with:

```powershell
mcfc build examples/<name> --clean
```

Then copy `examples/<name>/dist` into `<world>/datapacks/` and run `/reload`.

## Feature demo

`examples/feature_demo_263` runs a self-check the first time a player is online. It prints `MCFCTEST PASS|FAIL <name>` lines with `/say`, so the results end up in `latest.log`. Run it again with `/function mcfc_demo:run_tests`.

::: details Source
<<< @/../examples/feature_demo_263/src/main.mcf{mcfc}
:::
