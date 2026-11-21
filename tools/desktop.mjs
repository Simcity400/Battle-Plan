import { spawn } from "node:child_process";
import { access, copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { cargoTarget } from "./cargo-target.mjs";
import { thirdPartyNotices } from "./third-party-notices.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const mode = process.argv[2];
let interrupted = false;
if (!["dev", "build"].includes(mode)) {
  console.error("Usage: node tools/desktop.mjs dev|build");
  process.exit(2);
}

const SHIPPED = JSON.parse(await readFile(path.join(root, "tools", "shipped.json"), "utf8"));

const STAGE = "stage";

async function shipped(source, target, into) {
  if (!(await stat(source)).isDirectory()) {
    const staged = path.join(root, "crates", "desktop", STAGE, target);
    await mkdir(path.dirname(staged), { recursive: true });
    await copyFile(source, staged);
    const name = target.split(path.sep).join("/");
    into[`${STAGE}/${name}`] = name;
    return;
  }
  for (const entry of await readdir(source)) await shipped(path.join(source, entry), path.join(target, entry), into);
}

function run(executable, args, cwd = root, capture = false) {
  if (interrupted) return Promise.reject(new Error("Interrupted"));
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { cwd, stdio: capture ? ["inherit", "pipe", "inherit"] : "inherit", windowsHide: true });
    let stdout = "";
    child.stdout?.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
    const interrupt = () => {
      interrupted = true;
      if (process.platform !== "win32") child.kill("SIGINT");
    };
    process.once("SIGINT", interrupt);
    child.once("error", (error) => {
      process.removeListener("SIGINT", interrupt);
      reject(error);
    });
    child.once("close", (code, signal) => {
      process.removeListener("SIGINT", interrupt);
      if (interrupted) reject(new Error("Interrupted"));
      else if (code === 0) resolve(stdout);
      else reject(new Error(`${path.basename(executable)} exited with ${code ?? signal}`));
    });
  });
}

function flagValue(args, ...names) {
  for (const [index, arg] of args.entries()) {
    for (const name of names) {
      if (arg === name) return args[index + 1];
      if (name.startsWith("--") && arg.startsWith(`${name}=`)) return arg.slice(name.length + 1);
    }
  }
  return undefined;
}

const extra = process.argv.slice(3);
const separator = extra.indexOf("--");
const cliArgs = separator < 0 ? extra : extra.slice(0, separator);
const runnerArgs = separator < 0 ? [] : extra.slice(separator + 1);
const appSeparator = runnerArgs.indexOf("--");
const cargoArgs = appSeparator < 0 ? runnerArgs : runnerArgs.slice(0, appSeparator);
const explicitProfile = flagValue(cargoArgs, "--profile");
const release = mode === "build"
  ? !(cliArgs.includes("--debug") || cliArgs.includes("-d"))
  : cliArgs.includes("--release") || cargoArgs.includes("--release") || cargoArgs.includes("-r");
const profile = explicitProfile ?? (mode === "build" && release ? "release-lto" : release ? "release" : "dev");
const target = flagValue(cliArgs, "--target", "-t") ?? flagValue(cargoArgs, "--target");
const targetArgs = target === undefined ? [] : ["--target", target];
const builtFor = target ?? await cargoTarget({ cwd: path.join(root, "crates", "desktop"), env: process.env, cargoArgs,
  home: process.env.CARGO_HOME ?? path.join(process.env.USERPROFILE ?? process.env.HOME ?? "", ".cargo") });
if (process.env.CARGO_TARGET_DIR) process.env.CARGO_TARGET_DIR = path.resolve(root, process.env.CARGO_TARGET_DIR);
const targetDir = flagValue(cargoArgs, "--target-dir");
if (targetDir === "" || (targetDir === undefined && cargoArgs.includes("--target-dir"))) {
  console.error("--target-dir needs a directory.");
  process.exit(2);
}
const targetDirArgs = targetDir === undefined ? [] : ["--target-dir", path.resolve(root, targetDir)];
const forwardedCargoArgs = explicitProfile === undefined && profile === "release-lto" ? ["--profile", profile] : [];
for (let index = 0; index < cargoArgs.length; index++) {
  const arg = cargoArgs[index];
  if (arg.startsWith("--profile=")) forwardedCargoArgs.push("--profile", arg.slice("--profile=".length));
  else if (arg.startsWith("--target-dir=")) forwardedCargoArgs.push(...targetDirArgs);
  else if (arg === "--target-dir") { forwardedCargoArgs.push(...targetDirArgs); index++; }
  else forwardedCargoArgs.push(arg);
}
const forwardedRunnerArgs = appSeparator < 0 ? forwardedCargoArgs : [...forwardedCargoArgs, ...runnerArgs.slice(appSeparator)];

try {
  const notices = mode === "build" ? await thirdPartyNotices(path.join(root, "LICENSES"), SHIPPED.notices.components) : null;
  const releaseVersion = mode === "build" ? process.env.BATTLE_PLAN_VERSION || undefined : undefined;
  if (releaseVersion) process.env.TAURI_CONFIG = JSON.stringify({ version: releaseVersion });
  if (mode === "build") {
    if (!process.env.npm_execpath) throw new Error("Run the build through npm: npm run build:desktop");
    await run(process.execPath, [process.env.npm_execpath, "run", "build:ui"]);
  }
  const metadata = JSON.parse(await run("cargo", ["metadata", "--no-deps", "--format-version", "1"], root, true));
  let output = path.join(targetDir === undefined ? metadata.target_directory : path.resolve(root, targetDir),
    ...(builtFor === undefined ? [] : [builtFor]), profile === "dev" ? "debug" : profile);
  if (mode === "build") {
    const embeds = new Set(metadata.packages.filter((member) => "custom-protocol" in member.features).map((member) => member.name));
    let messages = "";
    for (const name of SHIPPED.cargo) {
      messages += await run("cargo", ["build", "-p", name, ...(embeds.has(name) ? ["--features", `${name}/custom-protocol`] : []),
        "--profile", profile, ...targetArgs, ...targetDirArgs, "--message-format=json-render-diagnostics"], root, true);
    }
    const artifact = messages.split("\n").map((line) => { try { return JSON.parse(line); } catch { return null; } })
      .find((message) => message?.reason === "compiler-artifact"
        && message.filenames?.some((file) => SHIPPED.built.includes(path.basename(file))));
    if (!artifact) throw new Error(`Cargo did not report where it built ${SHIPPED.built.join(", ")}.`);
    output = path.dirname(artifact.filenames.find((file) => SHIPPED.built.includes(path.basename(file))));
  }
  const present = await Promise.all(SHIPPED.compiler.map((name) => access(path.join(output, name)).then(() => true, () => false)));
  if (mode === "build" || present.includes(false)) {
    await run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
      path.join(root, "tools", "install-shader-compiler.ps1"), "-Destination", output]);
  }
  const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  let config = { build: {
    beforeDevCommand: { script: "npm run dev:ui", cwd: root },
    beforeBuildCommand: { script: "npm run build:ui", cwd: root },
  } };
  if (mode === "build") {
    const resources = {};
    await rm(path.join(root, "crates", "desktop", STAGE), { recursive: true, force: true });
    for (const name of [...SHIPPED.built, ...SHIPPED.compiler]) await shipped(path.join(output, name), name, resources);
    for (const name of SHIPPED.repository) await shipped(path.join(root, name), name, resources);
    await writeFile(path.join(root, "crates", "desktop", STAGE, SHIPPED.notices.file), notices);
    resources[`${STAGE}/${SHIPPED.notices.file}`] = SHIPPED.notices.file;
    const desktop = metadata.packages.find((member) => path.relative(path.join(root, "crates", "desktop"), path.dirname(member.manifest_path)) === "");
    if (!desktop) throw new Error("Cargo metadata names no package in crates/desktop.");
    const nightly = (releaseVersion ?? desktop.version).includes("-nightly.");
    config = { ...(releaseVersion ? { version: releaseVersion } : {}), build: { beforeBuildCommand: null }, bundle: { resources,
      ...(nightly ? { icon: ["icons/nightly.ico", "icons/nightly.png"], windows: { nsis: { installerIcon: "icons/nightly.ico" } } } : {}) } };
  }
  const args = mode === "build"
    ? [...cliArgs, ...(separator < 0 && forwardedRunnerArgs.length === 0 ? [] : ["--", ...forwardedRunnerArgs])]
    : [...cliArgs, "--", ...forwardedRunnerArgs];
  await run(process.execPath, [cli, mode, "--config", JSON.stringify(config), ...args], path.join(root, "crates", "desktop"));
} catch (error) {
  if (!interrupted) console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = interrupted ? 130 : 1;
}
