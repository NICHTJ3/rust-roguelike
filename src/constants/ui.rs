use crate::constants::*;

// sizes and coordinates relevant for the GUI
pub const BAR_WIDTH: i32 = 20;
pub const PANEL_HEIGHT: i32 = 7;
pub const PANEL_Y: i32 = SCREEN_HEIGHT - PANEL_HEIGHT;
pub const MSG_X: i32 = BAR_WIDTH + 2;
pub const MSG_WIDTH: i32 = SCREEN_WIDTH - BAR_WIDTH - 2;
pub const MSG_HEIGHT: usize = PANEL_HEIGHT as usize - 1;
pub const INVENTORY_WIDTH: i32 = 50;
pub const CHARACTER_SCREEN_WIDTH: i32 = 30;
pub const LEVEL_SCREEN_WIDTH: i32 = 40;
