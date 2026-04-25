#!/usr/bin/env node
/**
 * HyperGate demo AB end-to-end test.
 *
 * This script intentionally controls processes through redirected stdin. It is
 * a manager-style test: the test process starts gateway / v1 / v2, owns their
 * stdin pipes, writes console commands, and verifies traffic by HTTP.
 *
 * @author sky
 */

const { spawn, spawnSync } = require("node:child_process");
const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const isWindows = process.platform === "win32";
const exeSuffix = isWindows ? ".exe" : "";
const targetDir = path.join(root, "target", "debug");
const gatewayExe = path.join(targetDir, `hypergate${exeSuffix}`);
const v1Exe = path.join(targetDir, `hypergate-version-v1${exeSuffix}`);
const v2Exe = path.join(targetDir, `hypergate-version-v2${exeSuffix}`);
const children = [];
const events = [];

main().catch((error) => {
  console.error(error.stack || String(error));
  process.exitCode = 1;
}).finally(async () => {
  await stopChildren();
});

async function main() {
  step("prepare", "link debug executables through cargo");
  relinkBinaries();
  assertExists(gatewayExe);
  assertExists(v1Exe);
  assertExists(v2Exe);
  pass("prepare", "debug executables are present");

  const beforeHypergateArtifacts = hypergateRuntimeExists();
  check("artifact", `.hypergate exists before run: ${beforeHypergateArtifacts}`);

  step("spawn", `start ${path.basename(v1Exe)}`);
  const v1 = spawnManaged(v1Exe, []);
  pass("spawn", `${path.basename(v1Exe)} pid=${v1.pid}`);

  step("spawn", `start ${path.basename(v2Exe)}`);
  const v2 = spawnManaged(v2Exe, []);
  pass("spawn", `${path.basename(v2Exe)} pid=${v2.pid}`);

  step("http", "GET http://127.0.0.1:9101/");
  const directV1 = await waitHttpExact("http://127.0.0.1:9101/", "HyperGate from hypergate-version-v1");
  pass("http", `v1 direct response: ${directV1}`);

  step("http", "GET http://127.0.0.1:9102/");
  const directV2 = await waitHttpExact("http://127.0.0.1:9102/", "HyperGate from hypergate-version-v2");
  pass("http", `v2 direct response: ${directV2}`);

  step("spawn", `start ${path.basename(gatewayExe)}`);
  const gateway = spawnManaged(gatewayExe, []);
  pass("spawn", `${path.basename(gatewayExe)} pid=${gateway.pid}`);

  step("http", "GET http://127.0.0.1:8080/ before switch");
  const gatewayInitial = await waitHttpExact("http://127.0.0.1:8080/", "HyperGate from hypergate-version-v1");
  pass("http", `gateway initial response: ${gatewayInitial}`);

  step("console", "write gateway command: version switch v2");
  writeConsole(gateway, "version switch v2");
  step("http", "GET http://127.0.0.1:8080/ after switch");
  const gatewayAfterSwitch = await waitHttpExact("http://127.0.0.1:8080/", "HyperGate from hypergate-version-v2");
  pass("http", `gateway switched response: ${gatewayAfterSwitch}`);

  step("console", "write gateway command: version rollback");
  writeConsole(gateway, "version rollback");
  step("http", "GET http://127.0.0.1:8080/ after rollback");
  const gatewayAfterRollback = await waitHttpExact("http://127.0.0.1:8080/", "HyperGate from hypergate-version-v1");
  pass("http", `gateway rollback response: ${gatewayAfterRollback}`);

  step("console", "write v1 command: app config");
  writeConsole(v1, "app config");
  step("console", "write v2 command: app config");
  writeConsole(v2, "app config");
  await waitOutputContains(v1, "Demo Config");
  pass("console", "v1 console returned Demo Config");
  await waitOutputContains(v2, "Demo Config");
  pass("console", "v2 console returned Demo Config");

  const afterHypergateArtifacts = hypergateRuntimeExists();
  check("artifact", `.hypergate exists after run: ${afterHypergateArtifacts}`);
  if (afterHypergateArtifacts) {
    throw new Error(".hypergate runtime artifacts were created; file control channel leaked back in");
  }

  section("summary");
  console.log(JSON.stringify({
    passed: true,
    control: "stdin owned by this test process",
    directV1,
    directV2,
    gatewayInitial,
    gatewayAfterSwitch,
    gatewayAfterRollback,
    beforeHypergateArtifacts,
    afterHypergateArtifacts,
    events,
  }, null, 2));
}

function relinkBinaries() {
  check("cargo", "cargo run --quiet --bin hypergate-version-v1 -- --help");
  runCargo(["run", "--quiet", "--bin", "hypergate-version-v1", "--", "--help"], { allowFailure: false });
  check("cargo", "cargo run --quiet --bin hypergate-version-v2 -- --help");
  runCargo(["run", "--quiet", "--bin", "hypergate-version-v2", "--", "--help"], { allowFailure: false });
  check("cargo", "cargo run --quiet -p hypergate --bin hypergate -- help");
  runCargo(["run", "--quiet", "-p", "hypergate", "--bin", "hypergate", "--", "help"], { allowFailure: false });
}

function runCargo(args, options) {
  const result = spawnSync("cargo", args, {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  });
  if (!options.allowFailure && result.status !== 0) {
    throw new Error(`cargo ${args.join(" ")} failed\n${result.stdout}\n${result.stderr}`);
  }
}

function spawnManaged(file, args) {
  const child = spawn(file, args, {
    cwd: root,
    env: {
      ...process.env,
      HYPERGATE_PLAIN_CONSOLE: "1",
    },
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  child.output = "";
  child.stderrOutput = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    child.output += chunk;
  });
  child.stderr.on("data", (chunk) => {
    child.stderrOutput += chunk;
  });
  children.push(child);
  return child;
}

function writeConsole(child, command) {
  child.stdin.write(`${command}\n`);
}

function section(title) {
  console.log(`\n${title.toUpperCase()}`);
}

function step(scope, message) {
  record("STEP", scope, message);
}

function pass(scope, message) {
  record("PASS", scope, message);
}

function check(scope, message) {
  record("CHECK", scope, message);
}

function record(level, scope, message) {
  const event = {
    level,
    scope,
    message,
  };
  events.push(event);
  console.log(`[${level}] ${scope} ${message}`);
}

async function waitOutputContains(child, expected) {
  const started = Date.now();
  while (Date.now() - started < 5000) {
    if (child.output.includes(expected)) {
      return;
    }
    await sleep(50);
  }
  throw new Error(`process output did not contain ${JSON.stringify(expected)}\nstdout:\n${child.output}\nstderr:\n${child.stderrOutput}`);
}

async function waitHttpExact(url, expected) {
  let last = "";
  const started = Date.now();
  while (Date.now() - started < 15000) {
    try {
      last = (await httpGet(url)).trim();
      if (last === expected) {
        return last;
      }
    } catch {
      // The service may still be binding its port.
    }
    await sleep(100);
  }
  throw new Error(`timeout waiting for ${url}; expected ${JSON.stringify(expected)}, last ${JSON.stringify(last)}`);
}

function httpGet(url) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, { timeout: 1000 }, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        body += chunk;
      });
      response.on("end", () => {
        resolve(body);
      });
    });
    request.on("timeout", () => {
      request.destroy(new Error(`timeout: ${url}`));
    });
    request.on("error", reject);
  });
}

async function stopChildren() {
  for (const child of children.reverse()) {
    if (child.exitCode !== null) {
      continue;
    }
    try {
      child.stdin.end();
    } catch {
      // Process may already be exiting.
    }
    child.kill();
  }
  await sleep(250);
}

function assertExists(file) {
  if (!fs.existsSync(file)) {
    throw new Error(`missing executable: ${file}`);
  }
}

function hypergateRuntimeExists() {
  return fs.existsSync(path.join(root, ".hypergate"));
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
