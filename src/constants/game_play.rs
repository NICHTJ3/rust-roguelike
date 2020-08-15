use tcod::map::FovAlgorithm;

// experience and level-ups
pub const LEVEL_UP_BASE: i32 = 200;
pub const LEVEL_UP_FACTOR: i32 = 150;

pub const FOV_ALGO: FovAlgorithm = FovAlgorithm::Basic; // default FOV algorithm
pub const FOV_LIGHT_WALLS: bool = true; // light walls or not
pub const TORCH_RADIUS: i32 = 10;
