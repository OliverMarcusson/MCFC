const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const extensionRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(extensionRoot, "..", "..");

function platformTarget() {
  switch (process.platform) {
    case "win32":
      return { dir: "win32-x64", binary: "mcfc-lsp.exe" };
    case "linux":
      return { dir: "linux-x64", binary: "mcfc-lsp" };
    default:
      throw new Error(`Unsupported packaging platform: ${process.platform}`);
  }
}

const target = platformTarget();
const destinationDir = path.join(extensionRoot, "server", target.dir);
const destination = path.join(destinationDir, target.binary);

execFileSync("cargo", ["build", "--bin", "mcfc-lsp", "--release"], {
  cwd: repoRoot,
  stdio: "inherit",
});

const releaseServer = path.join(repoRoot, "target", "release", target.binary);
if (!fs.existsSync(releaseServer)) {
  throw new Error(`Expected built server at ${releaseServer}`);
}

fs.mkdirSync(destinationDir, { recursive: true });
fs.copyFileSync(releaseServer, destination);
if (process.platform !== "win32") {
  fs.chmodSync(destination, 0o755);
}
console.log(`Copied ${releaseServer} to ${destination}`);
