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

type ServerTarget = {
  platform: NodeJS.Platform;
  arch: string;
  dir: string;
  binary: string;
};

const SUPPORTED_SERVER_TARGETS: readonly ServerTarget[] = [
  { platform: "linux", arch: "x64", dir: "linux-x64", binary: "mcfc-lsp" },
  { platform: "win32", arch: "x64", dir: "win32-x64", binary: "mcfc-lsp.exe" },
];

export function activate(context: vscode.ExtensionContext): void {
  let serverPath: string;
  try {
    serverPath = resolveServerPath(context.extensionPath);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    void vscode.window.showErrorMessage(`MCFC Language Support: ${message}`);
    throw error;
  }

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
  const target = currentServerTarget();
  const devBinary = process.platform === "win32" ? "mcfc-lsp.exe" : "mcfc-lsp";
  const devServer = path.resolve(
    extensionPath,
    "..",
    "..",
    "target",
    "debug",
    devBinary,
  );

  if (target !== undefined) {
    const packagedServer = path.join(extensionPath, "server", target.dir, target.binary);
    if (fs.existsSync(packagedServer)) {
      return packagedServer;
    }
  }

  if (fs.existsSync(devServer)) {
    return devServer;
  }

  if (target === undefined) {
    throw new Error(
      `unsupported runtime target ${process.platform}-${process.arch}. ` +
        `Bundled mcfc-lsp binaries are only provided for ${supportedTargetList()}. macOS is currently unsupported.`,
    );
  }

  throw new Error(
    `unable to find mcfc-lsp for ${target.dir}. ` +
      `This extension currently ships platform-specific VSIX packages; build/install the VSIX produced on ${target.dir}, ` +
      `or run from the repository with a local target/debug/${target.binary} build present.`,
  );
}

function currentServerTarget(): ServerTarget | undefined {
  return SUPPORTED_SERVER_TARGETS.find(
    (target) => target.platform === process.platform && target.arch === process.arch,
  );
}

function supportedTargetList(): string {
  return SUPPORTED_SERVER_TARGETS.map((target) => target.dir).join(", ");
}
