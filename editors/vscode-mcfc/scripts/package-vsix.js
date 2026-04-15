const { execFileSync } = require("child_process");
const path = require("path");

const extensionRoot = path.resolve(__dirname, "..");
const target = process.argv[2];

if (!target) {
  console.error("usage: node ./scripts/package-vsix.js <linux-x64|win32-x64>");
  process.exit(1);
}

const env = {
  ...process.env,
  MCFC_VSCODE_TARGET: target,
};

const npx = process.platform === "win32" ? "npx.cmd" : "npx";
execFileSync(
  npx,
  ["vsce", "package", "--target", target, "--ignore-other-target-folders"],
  {
    cwd: extensionRoot,
    stdio: "inherit",
    env,
  },
);
