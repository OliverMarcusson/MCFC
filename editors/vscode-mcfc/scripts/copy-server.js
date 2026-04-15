const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const extensionRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(extensionRoot, "..", "..");

const SUPPORTED_TARGETS = {
  "linux-x64": {
    dir: "linux-x64",
    binary: "mcfc-lsp",
    cargoTarget: "x86_64-unknown-linux-gnu",
  },
  "win32-x64": {
    dir: "win32-x64",
    binary: "mcfc-lsp.exe",
    cargoTarget: "x86_64-pc-windows-gnu",
  },
};

function hostTargetKey() {
  if (process.arch !== "x64") {
    throw new Error(
      `Unsupported packaging architecture: ${process.arch}. Only linux-x64 and win32-x64 are currently packaged.`,
    );
  }

  switch (process.platform) {
    case "win32":
      return "win32-x64";
    case "linux":
      return "linux-x64";
    default:
      throw new Error(
        `Unsupported packaging platform: ${process.platform}. macOS is not currently packaged.`,
      );
  }
}

function packageTarget() {
  const requestedTarget = process.env.MCFC_VSCODE_TARGET;
  if (requestedTarget !== undefined && requestedTarget.length > 0) {
    const selectedTarget = SUPPORTED_TARGETS[requestedTarget];
    if (!selectedTarget) {
      throw new Error(
        `Unsupported MCFC_VSCODE_TARGET=${requestedTarget}. Expected one of: ${Object.keys(SUPPORTED_TARGETS).join(", ")}`,
      );
    }
    return selectedTarget;
  }

  return SUPPORTED_TARGETS[hostTargetKey()];
}

const target = packageTarget();
const serverRoot = path.join(extensionRoot, "server");
const destinationDir = path.join(serverRoot, target.dir);
const destination = path.join(destinationDir, target.binary);

execFileSync(
  "cargo",
  ["build", "--bin", "mcfc-lsp", "--release", "--target", target.cargoTarget],
  {
    cwd: repoRoot,
    stdio: "inherit",
  },
);

const releaseServer = path.join(
  repoRoot,
  "target",
  target.cargoTarget,
  "release",
  target.binary,
);
if (!fs.existsSync(releaseServer)) {
  throw new Error(`Expected built server at ${releaseServer}`);
}

fs.rmSync(serverRoot, { recursive: true, force: true });
fs.mkdirSync(destinationDir, { recursive: true });
fs.copyFileSync(releaseServer, destination);
if (process.platform !== "win32") {
  fs.chmodSync(destination, 0o755);
}
console.log(`Copied ${releaseServer} to ${destination}`);
console.log(`Prepared a platform-specific VSIX payload for ${target.dir}.`);
