use std::cmp;

mod components;
use components::{ai::*, fighter::Fighter};

mod constants;
use constants::{
    colors::*, dungeon::*, game_play::*, spells::*, ui::*, LIMIT_FPS, MAP_HEIGHT, MAP_WIDTH,
    PLAYER, SCREEN_HEIGHT, SCREEN_WIDTH,
};

mod death_callback;
use death_callback::*;

mod equipment;
use equipment::{Equipment, Slot};

mod game;
use game::{FontLayout, FontType, FovMap, Game, Map, Offscreen, PlayerAction, Root, Tcod};

mod messages;
use messages::Messages;

mod object;
use object::{Item, Object};

mod rect;
use rect::Rect;

mod tile;
use tile::Tile;

mod stuff;
use stuff::*;

use rand::Rng;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Write};
use tcod::console::*;
use tcod::input::{self, Event, Key, Mouse};

/// add to the player's inventory and remove from the map
fn pick_item_up(object_id: usize, game: &mut Game, objects: &mut Vec<Object>) {
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

fn get_equipped_in_slot(slot: Slot, inventory: &[Object]) -> Option<usize> {
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

enum UseResult {
    UsedUp,
    UsedAndKept,
    Cancelled,
}

fn use_item(inventory_id: usize, tcod: &mut Tcod, game: &mut Game, objects: &mut [Object]) {
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

fn drop_item(inventory_id: usize, game: &mut Game, objects: &mut Vec<Object>) {
    let mut item = game.inventory.remove(inventory_id);
    if item.equipment.is_some() {
        item.dequip(&mut game.messages);
    }
    item.set_pos(objects[PLAYER].x, objects[PLAYER].y);
    game.messages
        .add(format!("You dropped a {}.", item.name), YELLOW);
    objects.push(item);
}

/// return the position of a tile left-clicked in player's FOV (optionally in a
/// range), or (None,None) if right-clicked.
fn target_tile(
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &[Object],
    max_range: Option<f32>,
) -> Option<(i32, i32)> {
    use tcod::input::KeyCode::Escape;
    loop {
        // render the screen. this erases the inventory and shows the names of
        // objects under the mouse.
        tcod.root.flush();
        let event = input::check_for_event(input::KEY_PRESS | input::MOUSE).map(|e| e.1);
        match event {
            Some(Event::Mouse(m)) => tcod.mouse = m,
            Some(Event::Key(k)) => tcod.key = k,
            None => tcod.key = Default::default(),
        }
        render_all(tcod, game, objects, false);

        let (x, y) = (tcod.mouse.cx as i32, tcod.mouse.cy as i32);

        // accept the target if the player clicked in FOV, and in case a range
        // is specified, if it's in that range
        let in_fov = (x < MAP_WIDTH) && (y < MAP_HEIGHT) && tcod.fov.is_in_fov(x, y);
        let in_range = max_range.map_or(true, |range| objects[PLAYER].distance(x, y) <= range);
        if tcod.mouse.lbutton_pressed && in_fov && in_range {
            return Some((x, y));
        }

        if tcod.mouse.rbutton_pressed || tcod.key.code == Escape {
            return None; // cancel if the player right-clicked or pressed Escape
        }
    }
}

/// returns a clicked monster inside FOV up to a range, or None if right-clicked
fn target_monster(
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &[Object],
    max_range: Option<f32>,
) -> Option<usize> {
    loop {
        match target_tile(tcod, game, objects, max_range) {
            Some((x, y)) => {
                // return the first clicked monster, otherwise continue looping
                for (id, obj) in objects.iter().enumerate() {
                    if obj.pos() == (x, y) && obj.fighter.is_some() && id != PLAYER {
                        return Some(id);
                    }
                }
            }
            None => return None,
        }
    }
}

/// find closest enemy, up to a maximum range, and in the player's FOV
fn closest_monster(tcod: &Tcod, objects: &[Object], max_range: i32) -> Option<usize> {
    let mut closest_enemy = None;
    let mut closest_dist = (max_range + 1) as f32; // start with (slightly more than) maximum range

    for (id, object) in objects.iter().enumerate() {
        if (id != PLAYER)
            && object.fighter.is_some()
            && object.ai.is_some()
            && tcod.fov.is_in_fov(object.x, object.y)
        {
            // calculate distance between this object and the player
            let dist = objects[PLAYER].distance_to(object);
            if dist < closest_dist {
                // it's closer, so remember it
                closest_enemy = Some(id);
                closest_dist = dist;
            }
        }
    }
    closest_enemy
}

fn cast_heal(
    _inventory_id: usize,
    _tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut [Object],
) -> UseResult {
    // heal the player
    let player = &mut objects[PLAYER];
    if let Some(fighter) = player.fighter {
        if fighter.hp == player.max_hp(game) {
            game.messages.add("You are already at full health.", RED);
            return UseResult::Cancelled;
        }
        game.messages
            .add("Your wounds start to feel better!", LIGHT_VIOLET);
        player.heal(HEAL_AMOUNT, game);
        return UseResult::UsedUp;
    }
    UseResult::Cancelled
}

fn cast_lightning(
    _inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut [Object],
) -> UseResult {
    // find closest enemy (inside a maximum range and damage it)
    let monster_id = closest_monster(tcod, objects, LIGHTNING_RANGE);
    if let Some(monster_id) = monster_id {
        // zap it!
        game.messages.add(
            format!(
                "A lightning bolt strikes the {} with a loud thunder! \
                 The damage is {} hit points.",
                objects[monster_id].name, LIGHTNING_DAMAGE
            ),
            LIGHT_BLUE,
        );
        if let Some(xp) = objects[monster_id].take_damage(LIGHTNING_DAMAGE, game) {
            objects[PLAYER].fighter.as_mut().unwrap().xp += xp;
        }
        UseResult::UsedUp
    } else {
        // no enemy found within maximum range
        game.messages
            .add("No enemy is close enough to strike.", RED);
        UseResult::Cancelled
    }
}

fn cast_confuse(
    _inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut [Object],
) -> UseResult {
    // ask the player for a target to confuse
    game.messages.add(
        "Left-click an enemy to confuse it, or right-click to cancel.",
        LIGHT_CYAN,
    );
    let monster_id = target_monster(tcod, game, objects, Some(CONFUSE_RANGE as f32));
    if let Some(monster_id) = monster_id {
        let old_ai = objects[monster_id].ai.take().unwrap_or(Ai::Basic);
        // replace the monster's AI with a "confused" one; after
        // some turns it will restore the old AI
        objects[monster_id].ai = Some(Ai::Confused {
            previous_ai: Box::new(old_ai),
            num_turns: CONFUSE_NUM_TURNS,
        });
        game.messages.add(
            format!(
                "The eyes of {} look vacant, as he starts to stumble around!",
                objects[monster_id].name
            ),
            LIGHT_GREEN,
        );
        UseResult::UsedUp
    } else {
        // no enemy fonud within maximum range
        game.messages
            .add("No enemy is close enough to strike.", RED);
        UseResult::Cancelled
    }
}

fn cast_fireball(
    _inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut [Object],
) -> UseResult {
    // ask the player for a target tile to throw a fireball at
    game.messages.add(
        "Left-click a target tile for the fireball, or right-click to cancel.",
        LIGHT_CYAN,
    );
    let (x, y) = match target_tile(tcod, game, objects, None) {
        Some(tile_pos) => tile_pos,
        None => return UseResult::Cancelled,
    };
    game.messages.add(
        format!(
            "The fireball explodes, burning everything within {} tiles!",
            FIREBALL_RADIUS
        ),
        ORANGE,
    );

    let mut xp_to_gain = 0;
    for (id, obj) in objects.iter_mut().enumerate() {
        if obj.distance(x, y) <= FIREBALL_RADIUS as f32 && obj.fighter.is_some() {
            game.messages.add(
                format!(
                    "The {} gets burned for {} hit points.",
                    obj.name, FIREBALL_DAMAGE
                ),
                ORANGE,
            );
            if let Some(xp) = obj.take_damage(FIREBALL_DAMAGE, game) {
                if id != PLAYER {
                    // Don't reward the player for burning themself!
                    xp_to_gain += xp;
                }
            }
        }
    }
    objects[PLAYER].fighter.as_mut().unwrap().xp += xp_to_gain;

    UseResult::UsedUp
}

fn toggle_equipment(
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

fn create_room(room: Rect, map: &mut Map) {
    // go through the tiles in the rectangle and make them passable
    for x in (room.x1 + 1)..room.x2 {
        for y in (room.y1 + 1)..room.y2 {
            map[x as usize][y as usize] = Tile::empty();
        }
    }
}

fn create_h_tunnel(x1: i32, x2: i32, y: i32, map: &mut Map) {
    // horizontal tunnel. `min()` and `max()` are used in case `x1 > x2`
    for x in cmp::min(x1, x2)..(cmp::max(x1, x2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn create_v_tunnel(y1: i32, y2: i32, x: i32, map: &mut Map) {
    // vertical tunnel
    for y in cmp::min(y1, y2)..(cmp::max(y1, y2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn make_map(objects: &mut Vec<Object>, level: u32) -> Map {
    // fill map with "blocked" tiles
    let mut map = vec![vec![Tile::wall(); MAP_HEIGHT as usize]; MAP_WIDTH as usize];

    // Player is the first element, remove everything else.
    // NOTE: works only when the player is the first object!
    assert_eq!(&objects[PLAYER] as *const _, &objects[0] as *const _);
    objects.truncate(1);

    let mut rooms = vec![];

    for _ in 0..MAX_ROOMS {
        // random width and height
        let w = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
        let h = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
        // random position without going out of the boundaries of the map
        let x = rand::thread_rng().gen_range(0, MAP_WIDTH - w);
        let y = rand::thread_rng().gen_range(0, MAP_HEIGHT - h);

        let new_room = Rect::new(x, y, w, h);

        // run through the other rooms and see if they intersect with this one
        let failed = rooms
            .iter()
            .any(|other_room| new_room.intersects_with(other_room));

        if !failed {
            // this means there are no intersections, so this room is valid

            // "paint" it to the map's tiles
            create_room(new_room, &mut map);

            // add some content to this room, such as monsters
            place_objects(new_room, &map, objects, level);

            // center coordinates of the new room, will be useful later
            let (new_x, new_y) = new_room.center();

            if rooms.is_empty() {
                // this is the first room, where the player starts at
                objects[PLAYER].set_pos(new_x, new_y);
            } else {
                // all rooms after the first:
                // connect it to the previous room with a tunnel

                // center coordinates of the previous room
                let (prev_x, prev_y) = rooms[rooms.len() - 1].center();

                // toss a coin (random bool value -- either true or false)
                if rand::random() {
                    // first move horizontally, then vertically
                    create_h_tunnel(prev_x, new_x, prev_y, &mut map);
                    create_v_tunnel(prev_y, new_y, new_x, &mut map);
                } else {
                    // first move vertically, then horizontally
                    create_v_tunnel(prev_y, new_y, prev_x, &mut map);
                    create_h_tunnel(prev_x, new_x, new_y, &mut map);
                }
            }

            // finally, append the new room to the list
            rooms.push(new_room);
        }
    }

    // create stairs at the center of the last room
    let (last_room_x, last_room_y) = rooms[rooms.len() - 1].center();
    let mut stairs = Object::new(last_room_x, last_room_y, '<', "stairs", WHITE, false);
    stairs.always_visible = true;
    objects.push(stairs);

    map
}

struct Transition {
    level: u32,
    value: u32,
}

/// Returns a value that depends on level. the table specifies what
/// value occurs after each level, default is 0.
fn from_dungeon_level(table: &[Transition], level: u32) -> u32 {
    table
        .iter()
        .rev()
        .find(|transition| level >= transition.level)
        .map_or(0, |transition| transition.value)
}

fn place_objects(room: Rect, map: &Map, objects: &mut Vec<Object>, level: u32) {
    use rand::distributions::{IndependentSample, Weighted, WeightedChoice};

    // maximum number of monsters per room
    let max_monsters = from_dungeon_level(
        &[
            Transition { level: 1, value: 2 },
            Transition { level: 4, value: 3 },
            Transition { level: 6, value: 5 },
        ],
        level,
    );

    // choose random number of monsters
    let num_monsters = rand::thread_rng().gen_range(0, max_monsters + 1);

    // monster random table
    let troll_chance = from_dungeon_level(
        &[
            Transition {
                level: 3,
                value: 15,
            },
            Transition {
                level: 5,
                value: 30,
            },
            Transition {
                level: 7,
                value: 60,
            },
        ],
        level,
    );

    let monster_chances = &mut [
        Weighted {
            weight: 80,
            item: "orc",
        },
        Weighted {
            weight: troll_chance,
            item: "troll",
        },
    ];
    let monster_choice = WeightedChoice::new(monster_chances);

    // maximum number of items per room
    let max_items = from_dungeon_level(
        &[
            Transition { level: 1, value: 1 },
            Transition { level: 4, value: 2 },
        ],
        level,
    );

    // item random table
    let item_chances = &mut [
        // healing potion always shows up, even if all other items have 0 chance
        Weighted {
            weight: 35,
            item: Item::Heal,
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 4,
                    value: 25,
                }],
                level,
            ),
            item: Item::Lightning,
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 6,
                    value: 25,
                }],
                level,
            ),
            item: Item::Fireball,
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 2,
                    value: 10,
                }],
                level,
            ),
            item: Item::Confuse,
        },
        Weighted {
            weight: from_dungeon_level(&[Transition { level: 4, value: 5 }], level),
            item: Item::Sword,
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 8,
                    value: 15,
                }],
                level,
            ),
            item: Item::Shield,
        },
    ];
    let item_choice = WeightedChoice::new(item_chances);

    for _ in 0..num_monsters {
        // choose random spot for this monster
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        // only place it if the tile is not blocked
        if !is_blocked(x, y, map, objects) {
            let mut monster = match monster_choice.ind_sample(&mut rand::thread_rng()) {
                "orc" => {
                    // create an orc
                    let mut orc = Object::new(x, y, 'o', "orc", DESATURATED_GREEN, true);
                    orc.fighter = Some(Fighter {
                        base_max_hp: 20,
                        hp: 20,
                        base_defense: 0,
                        base_power: 4,
                        xp: 35,
                        on_death: DeathCallback::Monster,
                    });
                    orc.ai = Some(Ai::Basic);
                    orc
                }
                "troll" => {
                    // create a troll
                    let mut troll = Object::new(x, y, 'T', "troll", DARKER_GREEN, true);
                    troll.fighter = Some(Fighter {
                        base_max_hp: 30,
                        hp: 30,
                        base_defense: 2,
                        base_power: 8,
                        xp: 100,
                        on_death: DeathCallback::Monster,
                    });
                    troll.ai = Some(Ai::Basic);
                    troll
                }
                _ => unreachable!(),
            };
            monster.alive = true;
            objects.push(monster);
        }
    }

    // choose random number of items
    let num_items = rand::thread_rng().gen_range(0, max_items + 1);

    for _ in 0..num_items {
        // choose random spot for this item
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        // only place it if the tile is not blocked
        if !is_blocked(x, y, map, objects) {
            let mut item = match item_choice.ind_sample(&mut rand::thread_rng()) {
                Item::Heal => {
                    // create a healing potion
                    let mut object = Object::new(x, y, '!', "healing potion", VIOLET, false);
                    object.item = Some(Item::Heal);
                    object
                }
                Item::Lightning => {
                    // create a lightning bolt scroll
                    let mut object =
                        Object::new(x, y, '#', "scroll of lightning bolt", LIGHT_YELLOW, false);
                    object.item = Some(Item::Lightning);
                    object
                }
                Item::Fireball => {
                    // create a fireball scroll
                    let mut object =
                        Object::new(x, y, '#', "scroll of fireball", LIGHT_YELLOW, false);
                    object.item = Some(Item::Fireball);
                    object
                }
                Item::Confuse => {
                    // create a confuse scroll
                    let mut object =
                        Object::new(x, y, '#', "scroll of confusion", LIGHT_YELLOW, false);
                    object.item = Some(Item::Confuse);
                    object
                }
                Item::Sword => {
                    // create a sword
                    let mut object = Object::new(x, y, '/', "sword", SKY, false);
                    object.item = Some(Item::Sword);
                    object.equipment = Some(Equipment {
                        equipped: false,
                        slot: Slot::RightHand,
                        max_hp_bonus: 0,
                        defense_bonus: 0,
                        power_bonus: 3,
                    });
                    object
                }
                Item::Shield => {
                    // create a shield
                    let mut object = Object::new(x, y, '[', "shield", DARKER_ORANGE, false);
                    object.item = Some(Item::Shield);
                    object.equipment = Some(Equipment {
                        equipped: false,
                        slot: Slot::LeftHand,
                        max_hp_bonus: 0,
                        defense_bonus: 1,
                        power_bonus: 0,
                    });
                    object
                }
            };
            item.always_visible = true;
            objects.push(item);
        }
    }
}

/// Advance to the next level
fn next_level(tcod: &mut Tcod, game: &mut Game, objects: &mut Vec<Object>) {
    game.messages.add(
        "You take a moment to rest, and recover your strength.",
        VIOLET,
    );
    let heal_hp = objects[PLAYER].max_hp(game) / 2;
    objects[PLAYER].heal(heal_hp, game);

    game.messages.add(
        "After a rare moment of peace, you descend deeper into \
         the heart of the dungeon...",
        RED,
    );
    game.dungeon_level += 1;
    game.map = make_map(objects, game.dungeon_level);
    initialise_fov(tcod, &game.map);
}

fn render_bar(
    panel: &mut Offscreen,
    x: i32,
    y: i32,
    total_width: i32,
    name: &str,
    value: i32,
    maximum: i32,
    bar_color: Color,
    back_color: Color,
) {
    // render a bar (HP, experience, etc). First calculate the width of the bar
    let bar_width = (value as f32 / maximum as f32 * total_width as f32) as i32;

    // render the background first
    panel.set_default_background(back_color);
    panel.rect(x, y, total_width, 1, false, BackgroundFlag::Screen);

    // now render the bar on top
    panel.set_default_background(bar_color);
    if bar_width > 0 {
        panel.rect(x, y, bar_width, 1, false, BackgroundFlag::Screen);
    }

    // finally, some centered text with the values
    panel.set_default_foreground(WHITE);
    panel.print_ex(
        x + total_width / 2,
        y,
        BackgroundFlag::None,
        TextAlignment::Center,
        &format!("{}: {}/{}", name, value, maximum),
    );
}

/// return a string with the names of all objects under the mouse
fn get_names_under_mouse(mouse: Mouse, objects: &[Object], fov_map: &FovMap) -> String {
    let (x, y) = (mouse.cx as i32, mouse.cy as i32);

    // create a list with the names of all objects at the mouse's coordinates and in FOV
    let names = objects
        .iter()
        .filter(|obj| obj.pos() == (x, y) && fov_map.is_in_fov(obj.x, obj.y))
        .map(|obj| obj.name.clone())
        .collect::<Vec<_>>();

    names.join(", ") // join the names, separated by commas
}

fn render_all(tcod: &mut Tcod, game: &mut Game, objects: &[Object], fov_recompute: bool) {
    if fov_recompute {
        // recompute FOV if needed (the player moved or something)
        let player = &objects[PLAYER];
        tcod.fov
            .compute_fov(player.x, player.y, TORCH_RADIUS, FOV_LIGHT_WALLS, FOV_ALGO);
    }

    // go through all tiles, and set their background color
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let visible = tcod.fov.is_in_fov(x, y);
            let wall = game.map[x as usize][y as usize].block_sight;
            let color = match (visible, wall) {
                // outside of field of view:
                (false, true) => COLOR_DARK_WALL,
                (false, false) => COLOR_DARK_GROUND,
                // inside fov:
                (true, true) => COLOR_LIGHT_WALL,
                (true, false) => COLOR_LIGHT_GROUND,
            };

            let explored = &mut game.map[x as usize][y as usize].explored;
            if visible {
                // since it's visible, explore it
                *explored = true;
            }
            if *explored {
                // show explored tiles only (any visible tile is explored already)
                tcod.con
                    .set_char_background(x, y, color, BackgroundFlag::Set);
            }
        }
    }

    let mut to_draw: Vec<_> = objects
        .iter()
        .filter(|o| {
            tcod.fov.is_in_fov(o.x, o.y)
                || (o.always_visible && game.map[o.x as usize][o.y as usize].explored)
        })
        .collect();
    // sort so that non-blocknig objects come first
    to_draw.sort_by(|o1, o2| o1.blocks.cmp(&o2.blocks));
    // draw the objects in the list
    for object in &to_draw {
        object.draw(&mut tcod.con);
    }

    // blit the contents of "con" to the root console
    blit(
        &tcod.con,
        (0, 0),
        (MAP_WIDTH, MAP_HEIGHT),
        &mut tcod.root,
        (0, 0),
        1.0,
        1.0,
    );

    // prepare to render the GUI panel
    tcod.panel.set_default_background(BLACK);
    tcod.panel.clear();

    // print the game messages, one line at a time
    let mut y = MSG_HEIGHT as i32;
    for &(ref msg, color) in game.messages.iter().rev() {
        let msg_height = tcod.panel.get_height_rect(MSG_X, y, MSG_WIDTH, 0, msg);
        y -= msg_height;
        if y < 0 {
            break;
        }
        tcod.panel.set_default_foreground(color);
        tcod.panel.print_rect(MSG_X, y, MSG_WIDTH, 0, msg);
    }

    // show the player's stats
    let hp = objects[PLAYER].fighter.map_or(0, |f| f.hp);
    let max_hp = objects[PLAYER].max_hp(game);
    render_bar(
        &mut tcod.panel,
        1,
        1,
        BAR_WIDTH,
        "HP",
        hp,
        max_hp,
        LIGHT_RED,
        DARKER_RED,
    );

    tcod.panel.print_ex(
        1,
        3,
        BackgroundFlag::None,
        TextAlignment::Left,
        format!("Dungeon level: {}", game.dungeon_level),
    );

    // display names of objects under the mouse
    tcod.panel.set_default_foreground(LIGHT_GREY);
    tcod.panel.print_ex(
        1,
        0,
        BackgroundFlag::None,
        TextAlignment::Left,
        get_names_under_mouse(tcod.mouse, objects, &tcod.fov),
    );

    // blit the contents of `panel` to the root console
    blit(
        &tcod.panel,
        (0, 0),
        (SCREEN_WIDTH, PANEL_HEIGHT),
        &mut tcod.root,
        (0, PANEL_Y),
        1.0,
        1.0,
    );
}

fn player_move_or_attack(dx: i32, dy: i32, game: &mut Game, objects: &mut [Object]) {
    // the coordinates the player is moving to/attacking
    let x = objects[PLAYER].x + dx;
    let y = objects[PLAYER].y + dy;

    // try to find an attackable object there
    let target_id = objects
        .iter()
        .position(|object| object.fighter.is_some() && object.pos() == (x, y));

    // attack if target found, move otherwise
    match target_id {
        Some(target_id) => {
            let (player, target) = mut_two(PLAYER, target_id, objects);
            player.attack(target, game);
        }
        None => {
            move_by(PLAYER, dx, dy, &game.map, objects);
        }
    }
}

fn menu<T: AsRef<str>>(header: &str, options: &[T], width: i32, root: &mut Root) -> Option<usize> {
    assert!(
        options.len() <= 26,
        "Cannot have a menu with more than 26 options."
    );

    // calculate total height for the header (after auto-wrap) and one line per option
    let header_height = if header.is_empty() {
        0
    } else {
        root.get_height_rect(0, 0, width, SCREEN_HEIGHT, header)
    };
    let height = options.len() as i32 + header_height;

    // create an off-screen console that represents the menu's window
    let mut window = Offscreen::new(width, height);

    // print the header, with auto-wrap
    window.set_default_foreground(WHITE);
    window.print_rect_ex(
        0,
        0,
        width,
        height,
        BackgroundFlag::None,
        TextAlignment::Left,
        header,
    );

    // print all the options
    for (index, option_text) in options.iter().enumerate() {
        let menu_letter = (b'a' + index as u8) as char;
        let text = format!("({}) {}", menu_letter, option_text.as_ref());
        window.print_ex(
            0,
            header_height + index as i32,
            BackgroundFlag::None,
            TextAlignment::Left,
            text,
        );
    }

    // blit the contents of "window" to the root console
    let x = SCREEN_WIDTH / 2 - width / 2;
    let y = SCREEN_HEIGHT / 2 - height / 2;
    blit(&window, (0, 0), (width, height), root, (x, y), 1.0, 0.7);

    // present the root console to the player and wait for a key-press
    root.flush();
    let key = root.wait_for_keypress(true);

    // convert the ASCII code to an index; if it corresponds to an option, return it
    if key.printable.is_alphabetic() {
        let index = key.printable.to_ascii_lowercase() as usize - 'a' as usize;
        if index < options.len() {
            Some(index)
        } else {
            None
        }
    } else {
        None
    }
}

fn inventory_menu(inventory: &[Object], header: &str, root: &mut Root) -> Option<usize> {
    // how a menu with each item of the inventory as an option
    let options = if inventory.len() == 0 {
        vec!["Inventory is empty.".into()]
    } else {
        inventory
            .iter()
            .map(|item| {
                // show additional information, in case it's equipped
                match item.equipment {
                    Some(equipment) if equipment.equipped => {
                        format!("{} (on {})", item.name, equipment.slot)
                    }
                    _ => item.name.clone(),
                }
            })
            .collect()
    };

    let inventory_index = menu(header, &options, INVENTORY_WIDTH, root);

    // if an item was chosen, return it
    if inventory.len() > 0 {
        inventory_index
    } else {
        None
    }
}

fn msgbox(text: &str, width: i32, root: &mut Root) {
    let options: &[&str] = &[];
    menu(text, options, width, root);
}

fn handle_keys(tcod: &mut Tcod, game: &mut Game, objects: &mut Vec<Object>) -> PlayerAction {
    use tcod::input::KeyCode::*;
    use PlayerAction::*;

    let player_alive = objects[PLAYER].alive;
    match (tcod.key, tcod.key.text(), player_alive) {
        (
            Key {
                code: Enter,
                alt: true,
                ..
            },
            _,
            _,
        ) => {
            // Alt+Enter: toggle fullscreen
            let fullscreen = tcod.root.is_fullscreen();
            tcod.root.set_fullscreen(!fullscreen);
            DidntTakeTurn
        }
        (Key { code: Escape, .. }, _, _) => Exit, // exit game

        // movement keys
        (Key { code: Up, .. }, _, true) | (Key { code: NumPad8, .. }, _, true) => {
            player_move_or_attack(0, -1, game, objects);
            TookTurn
        }
        (Key { code: Down, .. }, _, true) | (Key { code: NumPad2, .. }, _, true) => {
            player_move_or_attack(0, 1, game, objects);
            TookTurn
        }
        (Key { code: Left, .. }, _, true) | (Key { code: NumPad4, .. }, _, true) => {
            player_move_or_attack(-1, 0, game, objects);
            TookTurn
        }
        (Key { code: Right, .. }, _, true) | (Key { code: NumPad6, .. }, _, true) => {
            player_move_or_attack(1, 0, game, objects);
            TookTurn
        }
        (Key { code: Home, .. }, _, true) | (Key { code: NumPad7, .. }, _, true) => {
            player_move_or_attack(-1, -1, game, objects);
            TookTurn
        }
        (Key { code: PageUp, .. }, _, true) | (Key { code: NumPad9, .. }, _, true) => {
            player_move_or_attack(1, -1, game, objects);
            TookTurn
        }
        (Key { code: End, .. }, _, true) | (Key { code: NumPad1, .. }, _, true) => {
            player_move_or_attack(-1, 1, game, objects);
            TookTurn
        }
        (Key { code: PageDown, .. }, _, true) | (Key { code: NumPad3, .. }, _, true) => {
            player_move_or_attack(1, 1, game, objects);
            TookTurn
        }
        (Key { code: NumPad5, .. }, _, true) => {
            TookTurn // do nothing, i.e. wait for the monster to come to you
        }

        (Key { code: Text, .. }, "g", true) => {
            // pick up an item
            let item_id = objects
                .iter()
                .position(|object| object.pos() == objects[PLAYER].pos() && object.item.is_some());
            if let Some(item_id) = item_id {
                pick_item_up(item_id, game, objects);
            }
            DidntTakeTurn
        }

        (Key { code: Text, .. }, "i", true) => {
            // show the inventory: if an item is selected, use it
            let inventory_index = inventory_menu(
                &game.inventory,
                "Press the key next to an item to use it, or any other to cancel.\n",
                &mut tcod.root,
            );
            if let Some(inventory_index) = inventory_index {
                use_item(inventory_index, tcod, game, objects);
            }
            DidntTakeTurn
        }

        (Key { code: Text, .. }, "d", true) => {
            // show the inventory; if an item is selected, drop it
            let inventory_index = inventory_menu(
                &game.inventory,
                "Press the key next to an item to drop it, or any other to cancel.\n'",
                &mut tcod.root,
            );
            if let Some(inventory_index) = inventory_index {
                drop_item(inventory_index, game, objects);
            }
            DidntTakeTurn
        }

        (Key { code: Text, .. }, "<", true) => {
            // go down stairs, if the player is on them
            let player_on_stairs = objects
                .iter()
                .any(|object| object.pos() == objects[PLAYER].pos() && object.name == "stairs");
            if player_on_stairs {
                next_level(tcod, game, objects);
            }
            DidntTakeTurn
        }

        (Key { code: Text, .. }, "c", true) => {
            // show character information
            let player = &objects[PLAYER];
            let level = player.level;
            let level_up_xp = LEVEL_UP_BASE + player.level * LEVEL_UP_FACTOR;
            if let Some(fighter) = player.fighter.as_ref() {
                let msg = format!(
                    "Character information

Level: {}
Experience: {}
Experience to level up: {}

Maximum HP: {}
Attack: {}
Defense: {}",
                    level,
                    fighter.xp,
                    level_up_xp,
                    player.max_hp(game),
                    player.power(game),
                    player.defense(game)
                );
                msgbox(&msg, CHARACTER_SCREEN_WIDTH, &mut tcod.root);
            }

            DidntTakeTurn
        }

        _ => DidntTakeTurn,
    }
}

fn level_up(tcod: &mut Tcod, game: &mut Game, objects: &mut [Object]) {
    let player = &mut objects[PLAYER];
    let level_up_xp = LEVEL_UP_BASE + player.level * LEVEL_UP_FACTOR;
    // see if the player's experience is enough to level-up
    if player.fighter.as_ref().map_or(0, |f| f.xp) >= level_up_xp {
        // it is! level up
        player.level += 1;
        game.messages.add(
            format!(
                "Your battle skills grow stronger! You reached level {}!",
                player.level
            ),
            YELLOW,
        );
        let fighter = player.fighter.as_mut().unwrap();
        let mut choice = None;
        while choice.is_none() {
            // keep asking until a choice is made
            choice = menu(
                "Level up! Choose a stat to raise:\n",
                &[
                    format!("Constitution (+20 HP, from {})", fighter.base_max_hp),
                    format!("Strength (+1 attack, from {})", fighter.base_power),
                    format!("Agility (+1 defense, from {})", fighter.base_defense),
                ],
                LEVEL_SCREEN_WIDTH,
                &mut tcod.root,
            );
        }
        fighter.xp -= level_up_xp;
        match choice.unwrap() {
            0 => {
                fighter.base_max_hp += 20;
                fighter.hp += 20;
            }
            1 => {
                fighter.base_power += 1;
            }
            2 => {
                fighter.base_defense += 1;
            }
            _ => unreachable!(),
        }
    }
}

fn new_game(tcod: &mut Tcod) -> (Game, Vec<Object>) {
    // create object representing the player
    let mut player = Object::new(0, 0, '@', "player", WHITE, true);
    player.alive = true;
    player.fighter = Some(Fighter {
        base_max_hp: 100,
        hp: 100,
        base_defense: 1,
        base_power: 2,
        xp: 0,
        on_death: DeathCallback::Player,
    });

    // the list of objects with just the player
    let mut objects = vec![player];

    let mut game = Game {
        // generate map (at this point it's not drawn to the screen)
        map: make_map(&mut objects, 1),
        messages: Messages::new(),
        inventory: vec![],
        dungeon_level: 1,
    };

    // initial equipment: a dagger
    let mut dagger = Object::new(0, 0, '-', "dagger", SKY, false);
    dagger.item = Some(Item::Sword);
    dagger.equipment = Some(Equipment {
        equipped: true,
        slot: Slot::LeftHand,
        max_hp_bonus: 0,
        defense_bonus: 0,
        power_bonus: 2,
    });
    game.inventory.push(dagger);

    initialise_fov(tcod, &game.map);

    // a warm welcoming message!
    game.messages.add(
        "Welcome stranger! Prepare to perish in the Tombs of the Ancient Kings.",
        RED,
    );

    (game, objects)
}

fn initialise_fov(tcod: &mut Tcod, map: &Map) {
    // create the FOV map, according to the generated map
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            tcod.fov.set(
                x,
                y,
                !map[x as usize][y as usize].block_sight,
                !map[x as usize][y as usize].blocked,
            );
        }
    }

    // unexplored areas start black (which is the default background color)
    tcod.con.clear();
}

fn play_game(tcod: &mut Tcod, game: &mut Game, objects: &mut Vec<Object>) {
    // force FOV "recompute" first time through the game loop
    let mut previous_player_position = (-1, -1);

    while !tcod.root.window_closed() {
        // clear the screen of the previous frame
        tcod.con.clear();

        match input::check_for_event(input::MOUSE | input::KEY_PRESS) {
            Some((_, Event::Mouse(m))) => tcod.mouse = m,
            Some((_, Event::Key(k))) => tcod.key = k,
            _ => tcod.key = Default::default(),
        }

        // render the screen
        let fov_recompute = previous_player_position != (objects[PLAYER].pos());
        render_all(tcod, game, &objects, fov_recompute);

        tcod.root.flush();

        // level up if needed
        level_up(tcod, game, objects);

        // handle keys and exit game if needed
        previous_player_position = objects[PLAYER].pos();
        let player_action = handle_keys(tcod, game, objects);
        if player_action == PlayerAction::Exit {
            save_game(game, objects).unwrap();
            break;
        }

        // let monsters take their turn
        if objects[PLAYER].alive && player_action != PlayerAction::DidntTakeTurn {
            for id in 0..objects.len() {
                if objects[id].ai.is_some() {
                    ai_take_turn(id, tcod, game, objects);
                }
            }
        }
    }
}

fn save_game(game: &Game, objects: &[Object]) -> Result<(), Box<dyn Error>> {
    let save_data = serde_json::to_string(&(game, objects))?;
    let mut file = File::create("savegame")?;
    file.write_all(save_data.as_bytes())?;
    Ok(())
}

fn load_game() -> Result<(Game, Vec<Object>), Box<dyn Error>> {
    let mut json_save_state = String::new();
    let mut file = File::open("savegame")?;
    file.read_to_string(&mut json_save_state)?;
    let result = serde_json::from_str::<(Game, Vec<Object>)>(&json_save_state)?;
    Ok(result)
}

fn main_menu(tcod: &mut Tcod) {
    let img = tcod::image::Image::from_file("menu_background.png")
        .ok()
        .expect("Background image not found");

    while !tcod.root.window_closed() {
        // show the background image, at twice the regular console resolution
        tcod::image::blit_2x(&img, (0, 0), (-1, -1), &mut tcod.root, (0, 0));

        tcod.root.set_default_foreground(LIGHT_YELLOW);
        tcod.root.print_ex(
            SCREEN_WIDTH / 2,
            SCREEN_HEIGHT / 2 - 4,
            BackgroundFlag::None,
            TextAlignment::Center,
            "TOMBS OF THE ANCIENT KINGS",
        );
        tcod.root.print_ex(
            SCREEN_WIDTH / 2,
            SCREEN_HEIGHT - 2,
            BackgroundFlag::None,
            TextAlignment::Center,
            "By Yours Truly",
        );

        // show options and wait for the player's choice
        let choices = &["Play a new game", "Continue last game", "Quit"];
        let choice = menu("", choices, 24, &mut tcod.root);

        match choice {
            Some(0) => {
                // new game
                let (mut game, mut objects) = new_game(tcod);
                play_game(tcod, &mut game, &mut objects);
            }
            Some(1) => {
                // load game
                match load_game() {
                    Ok((mut game, mut objects)) => {
                        initialise_fov(tcod, &game.map);
                        play_game(tcod, &mut game, &mut objects);
                    }
                    Err(_e) => {
                        msgbox("\nNo saved game to load.\n", 24, &mut tcod.root);
                        continue;
                    }
                }
            }
            Some(2) => {
                // quit
                break;
            }
            _ => {}
        }
    }
}

fn main() {
    tcod::system::set_fps(LIMIT_FPS);

    let root = Root::initializer()
        .font("arial10x10.png", FontLayout::Tcod)
        .font_type(FontType::Greyscale)
        .size(SCREEN_WIDTH, SCREEN_HEIGHT)
        .title("Rust/libtcod tutorial")
        .init();

    let mut tcod = Tcod {
        root,
        con: Offscreen::new(MAP_WIDTH, MAP_HEIGHT),
        panel: Offscreen::new(SCREEN_WIDTH, PANEL_HEIGHT),
        fov: FovMap::new(MAP_WIDTH, MAP_HEIGHT),
        key: Default::default(),
        mouse: Default::default(),
    };

    main_menu(&mut tcod);
}
