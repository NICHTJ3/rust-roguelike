pub mod colors;
pub mod dungeon;
pub mod game_play;
pub mod spells;
pub mod ui;

// player will always be the first object
pub const PLAYER: usize = 0;

// actual size of the window
pub const SCREEN_WIDTH: i32 = 80;
pub const SCREEN_HEIGHT: i32 = 50;

pub const LIMIT_FPS: i32 = 20; // 20 frames-per-second maximum

// size of the map
pub const MAP_WIDTH: i32 = SCREEN_WIDTH;
pub const MAP_HEIGHT: i32 = SCREEN_HEIGHT - ui::PANEL_HEIGHT;
