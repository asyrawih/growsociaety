//! Gameplay rules shared by client and server. Keep it engine-free.

pub mod claims;
pub mod commands;
pub mod farming;
pub mod inventory;
pub mod items;
pub mod levels;
pub mod mirror;
pub mod physics;
pub mod rng;
pub mod rules;
pub mod trade;
pub mod world;
pub mod worldgen;

pub const WORLD_WIDTH: u32 = 100;
pub const WORLD_HEIGHT: u32 = 50;
pub const TILE_SIZE: f32 = 32.0;

/// Tile grid position -> world-space center (y up, tile (0,0) bottom-left).
pub fn tile_to_world(x: u32, y: u32) -> (f32, f32) {
    ((x as f32 + 0.5) * TILE_SIZE, (y as f32 + 0.5) * TILE_SIZE)
}

/// World-space point -> tile grid position, `None` outside the world.
pub fn world_to_tile(wx: f32, wy: f32) -> Option<(u32, u32)> {
    let (x, y) = ((wx / TILE_SIZE).floor(), (wy / TILE_SIZE).floor());
    (x >= 0.0 && y >= 0.0 && x < WORLD_WIDTH as f32 && y < WORLD_HEIGHT as f32)
        .then_some((x as u32, y as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_bounds() {
        let (wx, wy) = tile_to_world(7, 3);
        assert_eq!(world_to_tile(wx, wy), Some((7, 3)));
        assert_eq!(world_to_tile(-1.0, 0.0), None);
        assert_eq!(world_to_tile(WORLD_WIDTH as f32 * TILE_SIZE, 0.0), None);
    }
}
