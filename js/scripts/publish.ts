import { $ } from "bun"
import packageJson from "../package.json"

const remoteVersion = await tryP($`npm view ${packageJson.name} version`.text(), "0.0.0")
const npmVersion = remoteVersion.split(".").map((str) => Number.parseInt(str))
const packageJsonVersion = packageJson.version.split(".").map((str) => Number.parseInt(str))

console.log("current npm deployed version: %q", remoteVersion)
console.log("current local package.json version: %q", packageJsonVersion)

let version = npmVersion
if (packageJsonVersion[0] > npmVersion[0]) {
  console.log("Local package.json MAJOR version is larger than deployed remote npm major version")
  version = packageJsonVersion
} else if (packageJsonVersion[1] > npmVersion[1]) {
  console.log("Local package.json MINOR version is larger than deployed remote npm minor version")
  version[1] = packageJsonVersion[1]
  version[2] = packageJsonVersion[2]
} else if (packageJsonVersion[2] > npmVersion[2]) {
  console.log("Local package.json PATCH version is larger than deployed remote npm minor version")
  version[2] = packageJsonVersion[2]
}

version[2]++
packageJson.version = version.join(".")

packageJson.exports["."] = {
  import: packageJson.exports["."].import.replace("src", "dist").replace(".ts", ".js"),
  types: packageJson.exports["."].types.replace("src", "dist").replace(".ts", ".d.ts"),
}

await $`echo ${JSON.stringify(packageJson, null, 2)} > package.json`
await $`bun run build`
await $`git tag "${packageJson.version}"`
await $`git push --tags`
await $`npm publish --no-git-checks`

async function tryP<T>(promise: Promise<T>, fallback: T) {
  try {
    return await promise
  } catch (error: unknown) {
    console.error(error)
    return fallback
  }
}
