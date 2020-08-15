use crate::{
    constants::{colors::*, PLAYER},
    game::{Game, Tcod},
    object::{Item, Object, UseResult},
    spells::*,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Slot {
    LeftHand,
    RightHand,
    Head,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
/// An object that can be equipped, yielding bonuses.
pub struct Equipment {
    pub slot: Slot,
    pub equipped: bool,
    pub max_hp_bonus: i32,
    pub defense_bonus: i32,
    pub power_bonus: i32,
}

/// add to the player's inventory and remove from the map
pub fn pick_item_up(object_id: usize, game: &mut Game, objects: &mut Vec<Object>) {
    if game.inventory.len() >= 26 {
        game.messages.add(
            format!(
                "Your inventory is full, cannot pick up {}.",
                objects[object_id].name
            ),
            RED,
        );
    } else {
        let item = objects.swap_remove(object_id);
        game.messages
            .add(format!("You picked up a {}!", item.name), GREEN);
        let index = game.inventory.len();
        let slot = item.equipment.map(|e| e.slot);
        game.inventory.push(item);

        // automatically equip, if the corresponding equipment slot is unused
        if let Some(slot) = slot {
            if get_equipped_in_slot(slot, &game.inventory).is_none() {
                game.inventory[index].equip(&mut game.messages);
            }
        }
    }
}

pub fn get_equipped_in_slot(slot: Slot, inventory: &[Object]) -> Option<usize> {
    for (inventory_id, item) in inventory.iter().enumerate() {
        if item
            .equipment
            .as_ref()
            .map_or(false, |e| e.equipped && e.slot == slot)
        {
            return Some(inventory_id);
        }
    }
    None
}

pub fn use_item(inventory_id: usize, tcod: &mut Tcod, game: &mut Game, objects: &mut [Object]) {
    use Item::*;
    // just call the "use_function" if it is defined
    if let Some(item) = game.inventory[inventory_id].item {
        let on_use = match item {
            Heal => cast_heal,
            Lightning => cast_lightning,
            Confuse => cast_confuse,
            Fireball => cast_fireball,
            Sword => toggle_equipment,
            Shield => toggle_equipment,
        };
        match on_use(inventory_id, tcod, game, objects) {
            UseResult::UsedUp => {
                // destroy after use, unless it was cancelled for some reason
                game.inventory.remove(inventory_id);
            }
            UseResult::UsedAndKept => {} // do nothing
            UseResult::Cancelled => {
                game.messages.add("Cancelled", WHITE);
            }
        }
    } else {
        game.messages.add(
            format!("The {} cannot be used.", game.inventory[inventory_id].name),
            WHITE,
        );
    }
}

pub fn drop_item(inventory_id: usize, game: &mut Game, objects: &mut Vec<Object>) {
    let mut item = game.inventory.remove(inventory_id);
    if item.equipment.is_some() {
        item.dequip(&mut game.messages);
    }
    item.set_pos(objects[PLAYER].x, objects[PLAYER].y);
    game.messages
        .add(format!("You dropped a {}.", item.name), YELLOW);
    objects.push(item);
}

pub fn toggle_equipment(
    inventory_id: usize,
    _tcod: &mut Tcod,
    game: &mut Game,
    _objects: &mut [Object],
) -> UseResult {
    let equipment = match game.inventory[inventory_id].equipment {
        Some(equipment) => equipment,
        None => return UseResult::Cancelled,
    };
    if equipment.equipped {
        game.inventory[inventory_id].dequip(&mut game.messages);
    } else {
        // if the slot is already being used, dequip whatever is there first
        if let Some(current) = get_equipped_in_slot(equipment.slot, &game.inventory) {
            game.inventory[current].dequip(&mut game.messages);
        }
        game.inventory[inventory_id].equip(&mut game.messages);
    }
    UseResult::UsedAndKept
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            Slot::LeftHand => write!(f, "left hand"),
            Slot::RightHand => write!(f, "right hand"),
            Slot::Head => write!(f, "head"),
        }
    }
}
