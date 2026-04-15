#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinecraftIdCategory {
    Block,
    Item,
    Entity,
    LootTable,
    Particle,
    SoundEvent,
    Effect,
}

include!(concat!(env!("OUT_DIR"), "/minecraft_ids.rs"));

pub fn ids_for_category(category: MinecraftIdCategory) -> &'static [&'static str] {
    match category {
        MinecraftIdCategory::Block => BLOCK_IDS,
        MinecraftIdCategory::Item => ITEM_IDS,
        MinecraftIdCategory::Entity => ENTITY_IDS,
        MinecraftIdCategory::LootTable => LOOT_TABLE_IDS,
        MinecraftIdCategory::Particle => PARTICLE_IDS,
        MinecraftIdCategory::SoundEvent => SOUND_EVENT_IDS,
        MinecraftIdCategory::Effect => EFFECT_IDS,
    }
}
