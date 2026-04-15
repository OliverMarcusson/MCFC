const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const extensionRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(extensionRoot, "..", "..");

function platformTarget() {
  if (process.arch !== "x64") {
    throw new Error(
      `Unsupported packaging architecture: ${process.arch}. Only linux-x64 and win32-x64 are currently packaged.`,
    );
  }

  switch (process.platform) {
    case "win32":
      return { dir: "win32-x64", binary: "mcfc-lsp.exe" };
    case "linux":
      return { dir: "linux-x64", binary: "mcfc-lsp" };
    default:
      throw new Error(
        `Unsupported packaging platform: ${process.platform}. macOS is not currently packaged.`,
      );
  }
}

const target = platformTarget();
const serverRoot = path.join(extensionRoot, "server");
const destinationDir = path.join(serverRoot, target.dir);
const destination = path.join(destinationDir, target.binary);

execFileSync("cargo", ["build", "--bin", "mcfc-lsp", "--release"], {
  cwd: repoRoot,
  stdio: "inherit",
});

const releaseServer = path.join(repoRoot, "target", "release", target.binary);
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
