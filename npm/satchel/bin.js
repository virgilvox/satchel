#!/usr/bin/env node

"use strict";

const { execFileSync } = require("child_process");
const path = require("path");
const os = require("os");

const PLATFORMS = {
  "darwin-arm64": "@satchel-rag/darwin-arm64",
  "darwin-x64": "@satchel-rag/darwin-x64",
  "linux-arm64": "@satchel-rag/linux-arm64",
  "linux-x64": "@satchel-rag/linux-x64",
  "win32-x64": "@satchel-rag/win32-x64",
};

function getBinaryPath() {
  const platform = os.platform();
  const arch = os.arch();
  const key = `${platform}-${arch}`;
  const pkg = PLATFORMS[key];

  if (!pkg) {
    throw new Error(
      `Unsupported platform: ${key}. ` +
      `Supported: ${Object.keys(PLATFORMS).join(", ")}`
    );
  }

  try {
    const pkgPath = require.resolve(`${pkg}/package.json`);
    const pkgDir = path.dirname(pkgPath);
    const binName = platform === "win32" ? "satchel.exe" : "satchel";
    return path.join(pkgDir, binName);
  } catch {
    throw new Error(
      `The platform package ${pkg} is not installed. ` +
      `Try reinstalling: npm install satchel-rag`
    );
  }
}

try {
  const binary = getBinaryPath();
  const result = execFileSync(binary, process.argv.slice(2), {
    stdio: "inherit",
    env: process.env,
  });
} catch (err) {
  if (err.status !== undefined) {
    process.exit(err.status);
  }
  console.error(err.message);
  process.exit(1);
}
