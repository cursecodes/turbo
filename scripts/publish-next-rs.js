#!/usr/bin/env node

const path = require("path");
const execa = require("execa");
const { copy } = require("fs-extra");
const { Sema } = require("async-sema");
const { readFile, readdir, writeFile } = require("fs/promises");

const cwd = process.cwd();

(async function () {
  try {
    const publishSema = new Sema(2);

    let version = `0.0.0-canary.${Date.now()}`;

    // Copy binaries to package folders, update version, and publish
    let nativePackagesDir = path.join(cwd, "crates/next-rs-napi/npm");
    let platforms = (await readdir(nativePackagesDir)).filter(
      (name) => !name.startsWith(".")
    );

    await Promise.all(
      platforms.map(async (platform) => {
        await publishSema.acquire();

        try {
          let binaryName = `next-swc.${platform}.node`;
          await copy(
            path.join(cwd, "packages/next-rs/native", binaryName),
            path.join(nativePackagesDir, platform, binaryName)
          );
          let pkg = JSON.parse(
            await readFile(
              path.join(nativePackagesDir, platform, "package.json")
            )
          );
          pkg.version = version;
          await writeFile(
            path.join(nativePackagesDir, platform, "package.json"),
            JSON.stringify(pkg, null, 2)
          );
          // await execa(
          //   `npm`,
          //   [
          //     `publish`,
          //     `${path.join(nativePackagesDir, platform)}`,
          //     `--access`,
          //     `public`,
          //     `--tag canary`,
          //     //...(version.includes("canary") ? ["--tag", "canary"] : []),
          //   ],
          //   { stdio: "inherit" }
          // );
        } catch (err) {
          // don't block publishing other versions on single platform error
          console.error(`Failed to publish`, platform, err);

          if (
            err.message &&
            err.message.includes(
              "You cannot publish over the previously published versions"
            )
          ) {
            console.error("Ignoring already published error", platform, err);
          } else {
            // throw err
          }
        } finally {
          publishSema.release();
        }
        // lerna publish in next step will fail if git status is not clean
        // await execa(
        //   `git`,
        //   [
        //     "update-index",
        //     "--skip-worktree",
        //     `${path.join(nativePackagesDir, platform, "package.json")}`,
        //   ],
        //   { stdio: "inherit" }
        // );
      })
    );

    // Update name/version of wasm packages and publish
    // let wasmDir = path.join(cwd, "crates/next-rs-wasm");

    // await Promise.all(
    //   ["web", "nodejs"].map(async (wasmTarget) => {
    //     await publishSema.acquire();

    //     let wasmPkg = JSON.parse(
    //       await readFile(path.join(wasmDir, `pkg-${wasmTarget}/package.json`))
    //     );
    //     wasmPkg.name = `@next/swc-wasm-${wasmTarget}`;
    //     wasmPkg.version = version;

    //     await writeFile(
    //       path.join(wasmDir, `pkg-${wasmTarget}/package.json`),
    //       JSON.stringify(wasmPkg, null, 2)
    //     );

    //     try {
    //       // await execa(
    //       //   `npm`,
    //       //   [
    //       //     "publish",
    //       //     `${path.join(wasmDir, `pkg-${wasmTarget}`)}`,
    //       //     "--access",
    //       //     "public",
    //       //     ...(version.includes("canary") ? ["--tag", "canary"] : []),
    //       //   ],
    //       //   { stdio: "inherit" }
    //       // );
    //     } catch (err) {
    //       // don't block publishing other versions on single platform error
    //       console.error(`Failed to publish`, wasmTarget, err);

    //       if (
    //         err.message &&
    //         err.message.includes(
    //           "You cannot publish over the previously published versions"
    //         )
    //       ) {
    //         console.error("Ignoring already published error", wasmTarget);
    //       } else {
    //         // throw err
    //       }
    //     } finally {
    //       publishSema.release();
    //     }
    //   })
    // );

    // Update optional dependencies versions
    let nextPkg = JSON.parse(
      await readFile(path.join(cwd, "packages/next-rs/package.json"))
    );
    for (let platform of platforms) {
      let optionalDependencies = nextPkg.optionalDependencies || {};
      optionalDependencies["@next/rs-" + platform] = version;
      nextPkg.optionalDependencies = optionalDependencies;
    }
    await writeFile(
      path.join(path.join(cwd, "packages/next-rs/package.json")),
      JSON.stringify(nextPkg, null, 2)
    );
    // lerna publish in next step will fail if git status is not clean
    // await execa(
    //   "git",
    //   ["update-index", "--skip-worktree", "packages/next/package.json"],
    //   {
    //     stdio: "inherit",
    //   }
    // );
  } catch (err) {
    console.error(err);
    process.exit(1);
  }
})();
