# Built-in Types

MCFC is statically typed. Local variables infer their type from their initializer; function parameters and return values are declared explicitly.

## Type List

| Type | Description |
| --- | --- |
| [`int`](./types/int) | Integer values and arithmetic. |
| [`bool`](./types/bool) | `true` or `false`; used by conditions and logical operators. |
| [`string`](./types/string) | Text values and Minecraft identifiers/selectors. |
| [`array<T>`](./types/array) | Ordered collection of values of type `T`. |
| [`dict<T>`](./types/dict) | String-keyed map of values of type `T`. |
| [`entity_set`](./types/entity-set) | A selector result that may contain multiple entities. |
| [`entity_ref`](./types/entity-ref) | A single entity reference. |
| [`player_ref`](./types/player-ref) | A single player reference. |
| [`block_ref`](./types/block-ref) | A block position reference. |
| [`entity_def`](./types/entity-def) | Mutable entity builder created with `entity(...)`. |
| [`block_def`](./types/block-def) | Mutable block builder created with `block_type(...)`. |
| [`item_def`](./types/item-def) | Mutable item builder created with `item(...)`. |
| [`text_def`](./types/text-def) | Mutable text component builder created with `text(...)`. |
| [`item_slot`](./types/item-slot) | Player inventory or hotbar slot surface. |
| [`bossbar`](./types/bossbar) | Bossbar handle created with `bossbar(...)`. |
| [`nbt`](./types/nbt) | NBT-like storage path or payload value. |
| [`void`](./types/void) | Function return type for no returned value. |
| [named `struct` types](./types/struct) | User-defined structured values. |

## Type Rules

- Assignments must keep the original variable type.
- Function arguments must match declared parameter types.
- Return expressions must match the declared return type.
- Arithmetic requires `int`.
- `and`, `or`, and `not` require `bool`.
- Ordering comparisons currently support `int` and `bool`.
- String equality supports `==` and `!=`.
- There are no broad implicit conversions.

Builder values have one important convenience: `entity_def`, `block_def`, and `item_def` can be used where an `nbt` payload is expected, as shorthand for `.as_nbt()`. `text_def` is storage-backed and can be assigned anywhere an NBT text component payload is expected.

## Examples

```mcfc
struct Reward:
    name: string
    amount: int

fn give_reward(player: player_ref, reward: Reward) -> void:
    let tags = ["quest", reward.name]
    let progress = {"steps": reward.amount}
    let sword = item("minecraft:diamond_sword")

    sword.name = reward.name
    player.give(sword)
    mcf "say tags=$(tags[0]), steps=$(progress[\"steps\"])"
```

```mcfc
fn place_marker() -> void:
    let pos = block("~ ~ ~")
    let marker = entity("minecraft:marker")
    marker.nbt.Tags[0] = "quest"

    pos.summon(marker)
```
