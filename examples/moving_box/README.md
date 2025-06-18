# Moving Box Example

This is just a bit further than a "hello world" style example. It displays a
small red square on the screen and moves it four different ways on a square
shaped path.

See the comments in `src/lib.rs` for further description and details.

## Concepts covered in this example

This example doesn't cover everything needed to create a game, but it's a good
place to learn the following concepts.

The exercises provided in the source are intended to be fun and help understand
these topics.

### Systems

- `system_once` runs once per game (at startup)
- Each `system` runs once per frame
- Each system specifies the data it may read or modify

### Resources

- A `resource` is initialized by the game engine
- Each Resource is a singleton (there's only one per game)

### Delta time

- Not every frame is exactly the same length of time
- The `MOVE_INCREMENT` (in [`src/lib.rs`]) is for each second of time
- The `delta_time` is how much of a second has past recently
- If we didn't scale by each specific frame time, the movement would appear move
  faster/slower and seem to make jerky movements.

Trivia: `delta_time` is a frame constant, meaning that for this frame it remains
the same, but it can be different from one frame to the next
