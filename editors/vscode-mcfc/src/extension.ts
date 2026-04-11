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
  const packagedServer = path.join(
    extensionPath,
    "server",
    "win32-x64",
    "mcfc-lsp.exe",
  );
  if (fs.existsSync(packagedServer)) {
    return packagedServer;
  }

  const devServer = path.resolve(
    extensionPath,
    "..",
    "..",
    "target",
    "debug",
    "mcfc-lsp.exe",
  );
  if (fs.existsSync(devServer)) {
    return devServer;
  }

  throw new Error(
    `Unable to find mcfc-lsp. Expected packaged server at ${packagedServer} or development server at ${devServer}.`,
  );
}
