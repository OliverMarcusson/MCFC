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

export function activate(context: vscode.ExtensionContext): void {
  const serverPath = resolveServerPath(context.extensionPath);
  const serverOptions: ServerOptions = {
    run: {
      command: serverPath,
      transport: TransportKind.stdio,
    },
    debug: {
      command: serverPath,
      transport: TransportKind.stdio,
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "mcfc" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.mcf"),
    },
  };

  client = new LanguageClient(
    "mcfc",
    "MCFC Language Server",
    serverOptions,
    clientOptions,
  );

  context.subscriptions.push(client);
  void client.start();
}

export function deactivate(): Thenable<void> | undefined {
  const activeClient = client;
  client = undefined;
  return activeClient?.stop();
}

function resolveServerPath(extensionPath: string): string {
  const platformTargets =
    process.platform === "win32"
      ? [{ dir: "win32-x64", binary: "mcfc-lsp.exe" }]
      : process.platform === "linux"
        ? [{ dir: "linux-x64", binary: "mcfc-lsp" }]
        : [];

  for (const target of platformTargets) {
    const packagedServer = path.join(
      extensionPath,
      "server",
      target.dir,
      target.binary,
    );
    if (fs.existsSync(packagedServer)) {
      return packagedServer;
    }
  }

  const devBinary = process.platform === "win32" ? "mcfc-lsp.exe" : "mcfc-lsp";
  const devServer = path.resolve(
    extensionPath,
    "..",
    "..",
    "target",
    "debug",
    devBinary,
  );
  if (fs.existsSync(devServer)) {
    return devServer;
  }

  const expectedPackagedServers = platformTargets
    .map((target) => path.join(extensionPath, "server", target.dir, target.binary))
    .join(", ");

  throw new Error(
    `Unable to find mcfc-lsp. Expected packaged server at one of [${expectedPackagedServers}] or development server at ${devServer}.`,
  );
}
