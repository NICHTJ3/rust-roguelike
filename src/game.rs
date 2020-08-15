use crate::{messages::Messages, object::Object, tile::Tile};
use serde::{Deserialize, Serialize};
pub use tcod::console::*;
use tcod::input::{Key, Mouse};
pub use tcod::map::Map as FovMap;

pub type Map = Vec<Vec<Tile>>;

pub struct Tcod {
    pub root: Root,
    pub con: Offscreen,
    pub panel: Offscreen,
    pub fov: FovMap,
    pub key: Key,
    pub mouse: Mouse,
}

#[derive(Serialize, Deserialize)]
pub struct Game {
    pub map: Map,
    pub messages: Messages,
    pub inventory: Vec<Object>,
    pub dungeon_level: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerAction {
    TookTurn,
    DidntTakeTurn,
    Exit,
}
