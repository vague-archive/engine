# Game Modules

## This directory

This directory holds modules provided with the Fiasco game engine. There are an
unlimited number of modules which could be created, so this is not an exhaustive
list.

You may want to use some, none, or all of these modules in your game. The choice
is yours!

When you create the logic for your game, that will also be a module.

## What is a module

A Fiasco module is implemented as a shared (or dynamic) library. It's not a
stand-alone program (there's no `main` entry point).

Modules are used by placing the built library into a `modules` directory in the
same directory as the platform executable.

Note: the `.dll` file extension is used on Windows, other platforms use
different filename extensions, but the concepts are equivalent.

E.g.
```
+ my_game/
+--+ modules
|  +--+ physics.dll
|  +--+ sound.dll
|  +--+ gameplay.dll
+--+ platform_native.exe
```

In the above example, the `platform_native.exe` will (at startup) look in the
`modules` directory and then load each of the three modules (in this example,
physics, sound, gameplay). Each game will have its own set of libraries (there
may be more or fewer modules than this example).
