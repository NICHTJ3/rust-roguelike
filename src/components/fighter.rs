// combat-related properties and methods (monster, player, NPC).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Fighter {
    hp: i32,
    base_max_hp: i32,
    base_defense: i32,
    base_power: i32,
    xp: i32,
    on_death: DeathCallback,
}
