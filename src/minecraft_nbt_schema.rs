#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NbtSchemaCategory {
    Entity,
    Block,
    Item,
}

#[derive(Debug)]
pub struct NbtSchemaNode {
    pub detail: &'static str,
    pub documentation: &'static str,
    pub fields: &'static [NbtSchemaField],
}

#[derive(Debug)]
pub struct NbtSchemaField {
    pub name: &'static str,
    pub detail: &'static str,
    pub documentation: &'static str,
    pub node: Option<&'static NbtSchemaNode>,
}

include!(concat!(env!("OUT_DIR"), "/minecraft_nbt_schema.rs"));

pub fn root_node(category: NbtSchemaCategory, id: Option<&str>) -> Option<&'static NbtSchemaNode> {
    generated_root(category, id)
}

pub fn child_node<'a>(node: &'a NbtSchemaNode, segment: &str) -> Option<&'a NbtSchemaNode> {
    node.fields
        .iter()
        .find(|field| field.name == segment)
        .and_then(|field| field.node)
}
