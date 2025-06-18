# Native JavaScript Entrypoint

This folder hold the TypeScript files needed for allowing JavaScript games to run natively.

There are two files:
- [entrypoint.ts](./src/entrypoint.ts) - The main module for any JS ECS module that is loaded. Registers the `engine` class globally.
- [extensions.ts](./src/extensions.ts) - Binds all the Rust functions we have exposed to the JS global variable called `Extension`.

## Requirements

- [Deno](https://deno.com)

Deno installation:

Mac | Linux
```sh
curl -fsSL https://deno.land/install.sh | sh
```

Windows
```sh
irm https://deno.land/install.ps1 | iex
```

## Formatting

This folder uses Deno's formating and linting.

Run with `deno fmt` and `deno lint`.