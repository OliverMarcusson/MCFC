import * as childProcess from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;
let output: vscode.OutputChannel | undefined;
const watches = new Map<string, childProcess.ChildProcess>();

type RuntimeTarget = { platform: NodeJS.Platform; arch: string; dir: string; lsp: string; cli: string };
const SUPPORTED_TARGETS: readonly RuntimeTarget[] = [
  { platform: "linux", arch: "x64", dir: "linux-x64", lsp: "mcfc-lsp", cli: "mcfc" },
  { platform: "win32", arch: "x64", dir: "win32-x64", lsp: "mcfc-lsp.exe", cli: "mcfc.exe" },
];

export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel("MCFC");
  context.subscriptions.push(output, { dispose: stopAllWatches });

  let serverPath: string;
  try { serverPath = resolveBinary(context.extensionPath, "lsp"); }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    void vscode.window.showErrorMessage(`MCFC Language Support: ${message}`);
    throw error;
  }

  const serverOptions: ServerOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio },
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "mcfc" },
      { scheme: "file", language: "toml", pattern: "**/mcfc.toml" },
    ],
    synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher("**/{*.mcf,mcfc.toml}") },
  };
  client = new LanguageClient("mcfc", "MCFC Language Server", serverOptions, clientOptions);
  context.subscriptions.push(client);
  void client.start();

  context.subscriptions.push(
    vscode.commands.registerCommand("mcfc.buildProject", () => runBuild(context, false)),
    vscode.commands.registerCommand("mcfc.watchProject", () => startWatch(context)),
    vscode.commands.registerCommand("mcfc.stopWatch", () => stopWatch(projectRoot())),
    vscode.commands.registerCommand("mcfc.deployProject", () => deployProject(context, false)),
    vscode.commands.registerCommand("mcfc.buildAndDeploy", () => deployProject(context, true)),
    vscode.commands.registerCommand("mcfc.openGeneratedDatapack", () => openGeneratedDatapack()),
  );
}

export function deactivate(): Thenable<void> | undefined {
  stopAllWatches();
  const activeClient = client; client = undefined;
  return activeClient?.stop();
}

function projectRoot(): string | undefined {
  const active = vscode.window.activeTextEditor?.document.uri.fsPath;
  const folder = active ? vscode.workspace.getWorkspaceFolder(vscode.Uri.file(active)) : vscode.workspace.workspaceFolders?.[0];
  return folder?.uri.fsPath;
}

function config() { return vscode.workspace.getConfiguration("mcfc"); }
function executable(context: vscode.ExtensionContext): string { return config().get<string>("cli.path") || resolveBinary(context.extensionPath, "cli"); }

function runBuild(context: vscode.ExtensionContext, quiet: boolean): Thenable<void> {
  const root = projectRoot();
  if (!root) { return Promise.reject(new Error("Open an MCFC workspace before building.")); }
  return runCommand(executable(context), ["build", root], root, quiet);
}

async function deployProject(context: vscode.ExtensionContext, buildFirst: boolean): Promise<void> {
  const root = projectRoot();
  if (!root) { throw new Error("Open an MCFC workspace before deploying."); }
  if (buildFirst) { await runBuild(context, true); }
  const manifest = path.join(root, "mcfc.toml");
  if (!fs.existsSync(manifest)) { throw new Error("Deploying requires mcfc.toml in the workspace root."); }
  const datapacks = config().get<string>("deploy.datapacksDirectory") || "";
  if (!datapacks) { throw new Error("Set mcfc.deploy.datapacksDirectory before deploying."); }
  const outDir = manifestValue(fs.readFileSync(manifest, "utf8"), "out_dir") || "dist";
  const namespace = manifestValue(fs.readFileSync(manifest, "utf8"), "namespace") || "mcfc";
  const packName = config().get<string>("deploy.packName") || namespace;
  const source = path.resolve(root, outDir);
  const destination = path.resolve(datapacks, packName);
  if (!fs.existsSync(source)) { throw new Error(`No generated datapack at ${source}. Build the project first.`); }
  if (path.dirname(destination) !== path.resolve(datapacks)) { throw new Error("MCFC deploy target must be a direct child of datapacksDirectory."); }
  fs.rmSync(destination, { recursive: true, force: true });
  fs.cpSync(source, destination, { recursive: true });
  log(`Deployed ${source} -> ${destination}`);
  const reload = config().get<string>("deploy.reloadCommand") || "";
  if (reload.trim()) {
    const command = reload.split("${datapackPath}").join(destination).split("${workspaceFolder}").join(root);
    await runShell(command, root);
  }
  void vscode.window.showInformationMessage(`MCFC deployed ${packName}.`);
}

function startWatch(context: vscode.ExtensionContext): void {
  const root = projectRoot();
  if (!root) { void vscode.window.showErrorMessage("Open an MCFC workspace before watching."); return; }
  stopWatch(root);
  const process = childProcess.spawn(executable(context), ["watch", root], { cwd: root, shell: false });
  watches.set(root, process);
  pipeProcess(process, "watch");
  void vscode.window.showInformationMessage("MCFC watch started.");
}

function stopWatch(root: string | undefined): void {
  if (!root) return;
  const process = watches.get(root); if (!process) return;
  process.kill(); watches.delete(root); log(`Stopped MCFC watch for ${root}`);
}
function stopAllWatches(): void { for (const root of [...watches.keys()]) stopWatch(root); }

async function openGeneratedDatapack(): Promise<void> {
  const root = projectRoot(); if (!root) throw new Error("Open an MCFC workspace first.");
  const manifest = path.join(root, "mcfc.toml");
  const outDir = fs.existsSync(manifest) ? manifestValue(fs.readFileSync(manifest, "utf8"), "out_dir") || "dist" : "dist";
  await vscode.commands.executeCommand("revealFileInOS", vscode.Uri.file(path.resolve(root, outDir)));
}

function runCommand(command: string, args: string[], cwd: string, quiet: boolean): Promise<void> {
  return new Promise((resolve, reject) => {
    const process = childProcess.spawn(command, args, { cwd, shell: false }); pipeProcess(process, `${path.basename(command)} ${args[0]}`);
    process.on("error", reject);
    process.on("exit", (code) => code === 0 ? resolve() : reject(new Error(`MCFC command exited with ${code ?? "an error"}.`)));
    if (!quiet) output?.show(true);
  });
}
function runShell(command: string, cwd: string): Promise<void> {
  return new Promise((resolve, reject) => childProcess.exec(command, { cwd }, (error, stdout, stderr) => {
    if (stdout) log(stdout); if (stderr) log(stderr); error ? reject(error) : resolve();
  }));
}
function pipeProcess(process: childProcess.ChildProcess, label: string): void {
  log(`$ ${label}`); process.stdout?.on("data", (data) => log(data.toString())); process.stderr?.on("data", (data) => log(data.toString()));
}
function log(value: string): void { output?.appendLine(value.trimEnd()); }
function manifestValue(text: string, key: string): string | undefined {
  return new RegExp(`^\\s*${key}\\s*=\\s*[\"']([^\"']+)[\"']`, "m").exec(text)?.[1];
}
function resolveBinary(extensionPath: string, kind: "lsp" | "cli"): string {
  const target = SUPPORTED_TARGETS.find((value) => value.platform === process.platform && value.arch === process.arch);
  const binary = kind === "lsp" ? (process.platform === "win32" ? "mcfc-lsp.exe" : "mcfc-lsp") : (process.platform === "win32" ? "mcfc.exe" : "mcfc");
  const dev = path.resolve(extensionPath, "..", "..", "target", "debug", binary);
  if (target) { const packaged = path.join(extensionPath, "server", target.dir, kind === "lsp" ? target.lsp : target.cli); if (fs.existsSync(packaged)) return packaged; }
  if (fs.existsSync(dev)) return dev;
  throw new Error(`unable to find ${binary} for ${process.platform}-${process.arch}; install a matching MCFC VSIX or configure mcfc.cli.path.`);
}
