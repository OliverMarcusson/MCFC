# Standard Library: `std`

Every file and project build can use the `std` module without declaring it. Import from it with [`import`](./statements#import) or call it by path:

```mcfc
import std.math.clamp;

void main() {
    var hp = clamp(150, 0, 100);
    var bits = std.math.pow(2, 10);
}
```

Only the `std` functions a pack calls are compiled into it. The name `std` is reserved, so a `src/std.mcf` file is an error.

## `std.math`

| Function | Returns |
| --- | --- |
| `int min(int a, int b)` | The smaller of `a` and `b`. |
| `int max(int a, int b)` | The larger of `a` and `b`. |
| `int abs(int x)` | `x` without its sign. |
| `int sign(int x)` | `1`, `0`, or `-1`. |
| `int clamp(int x, int low, int high)` | `x` limited to `low` through `high`. |
| `int rem(int a, int b)` | The remainder of `a / b`, with the sign of `b`. For example, `rem(-7, 3)` is `2`. Same as `a % b`. |
| `int pow(int base, int exponent)` | `base` multiplied by itself `exponent` times. Negative exponents return `0`. |
| `int gcd(int a, int b)` | The greatest common divisor, never negative. `gcd(12, -18)` is `6`. |
| `float lerp(float a, float b, float t)` | The point `t` of the way from `a` to `b`. `0.0` gives `a` and `1.0` gives `b`. |

Integer arithmetic is 32-bit scoreboard math, so results wrap on overflow.

## `std.list`

These are [generic](./statements#generic-functions), so they work on `List<Integer>` and `List<Float>`.

| Function | Returns |
| --- | --- |
| `<T> T sum(List<T> xs)` | The total of all elements, or `0` for an empty list. |
| `<T> T min(List<T> xs)` | The smallest element, or `0` for an empty list. |
| `<T> T max(List<T> xs)` | The largest element, or `0` for an empty list. |

To sort, use the built-in [`xs.sort()`](./types#list) method.

## `std.noise`

Perlin noise: smooth random-looking values for terrain, particle paths and motion. The same inputs always give the same value.

```mcfc
import std.noise;

void main() {
    var height = 64 + (int) (noise.fractal(10 * 0.05, 20 * 0.05, 4) * 12.0);
}
```

| Function | Returns |
| --- | --- |
| `float perlin(float x, float y)` | 2D Perlin noise, about -1 to 1, and 0 at whole-number points. Multiply the inputs by a small number such as `0.05` for larger features, and add an offset for a different pattern. |
| `float fractal(float x, float y, int octaves)` | `octaves` layers of `perlin` added up, each twice as detailed and half as strong, scaled back to about -1 to 1. |

Each `perlin` call costs a few hundred commands, so sample it once per block or entity, not every tick for many of them.

## `std.shape`

Filled shapes, one `fill` per column, so even large shapes cost a few hundred commands.

```mcfc
import std.shape;

void build() {
    shape.sphere(Block.of(0, 80, 0), 6, "minecraft:glass");
    shape.cylinder(Block.of(20, 64, 0), 3, 10, "minecraft:stone_bricks");
    shape.heightmap(Block.of(40, 60, 0), 16, 16, 8, 0.05, "minecraft:grass_block");
}
```

| Function | Fills |
| --- | --- |
| `sphere(Block center, int radius, String block)` | Every block within `radius` of `center`. |
| `cylinder(Block base, int radius, int height, String block)` | An upright cylinder standing on `base`. |
| `heightmap(Block corner, int width, int depth, int amplitude, float scale, String block)` | Terrain over a `width` by `depth` area: each column rises from `corner` by up to `amplitude` blocks, following [`noise.fractal`](#std-noise). `scale` sets the hill size; try `0.05`. |

Shapes replace what's there, like `fill`.

## `std.str`

| Function | Returns |
| --- | --- |
| `boolean startsWith(String s, String prefix)` | `true` when `s` begins with `prefix`. |
| `boolean endsWith(String s, String suffix)` | `true` when `s` ends with `suffix`. |
| `int find(String s, String needle)` | The index of the first `needle` in `s`, or `-1`. |
| `boolean contains(String s, String needle)` | `true` when `needle` appears in `s`. |
| `String replace(String s, String target, String replacement)` | `s` with every `target` replaced. |
| `List<String> split(String s, String separator)` | The parts between each `separator`, without trailing empty parts. |
| `String toUpperCase(String s)`, `String toLowerCase(String s)` | `s` with ASCII letters changed. |

The `String` methods `startsWith`, `endsWith`, `indexOf`, `contains`, `replace`, `split`, `toUpperCase` and `toLowerCase` call these helpers. Import `std.str` functions only when you need the free-function form. They compare substrings of `s`, so they cost a few commands per character. `startsWith`, `endsWith`, `find` and `contains` never paste the text into a command, so `"` and `\` are safe. The others build new strings by joining, which has the [joining limits](./types#string).

## `std.vec`

`Vec3` is a record of three floats, for velocities, directions and positions. Import the record and the module:

```mcfc
import std.vec;
import std.vec.Vec3;

void main() {
    var player = (Player) Selector.of("@p").getFirst();
    var look = new Vec3(player.getLookX(), player.getLookY(), player.getLookZ());
    var push = vec.scale(vec.normalize(vec.add(look, new Vec3(0.0, 1.0, 0.0))), 0.8);
    player.addVelocity(push.x(), push.y(), push.z());
}
```

| Function | Returns |
| --- | --- |
| `Vec3 add(Vec3 a, Vec3 b)`, `Vec3 sub(Vec3 a, Vec3 b)` | The component-wise sum or difference. |
| `Vec3 scale(Vec3 v, float factor)` | `v` with every component multiplied by `factor`. |
| `float dot(Vec3 a, Vec3 b)` | The dot product. |
| `Vec3 cross(Vec3 a, Vec3 b)` | The cross product, perpendicular to both. |
| `float length(Vec3 v)` | The length of `v`. |
| `Vec3 normalize(Vec3 v)` | `v` scaled to length 1. The zero vector stays zero. |

## `std.attribute`

`Attribute` names the attributes you change most often, for `getAttribute` and `setAttribute`: `MOVEMENT_SPEED`, `JUMP_STRENGTH`, `GRAVITY`, `STEP_HEIGHT`, `SCALE`, `SAFE_FALL_DISTANCE` and `KNOCKBACK_RESISTANCE`. Pass a string ID such as `"minecraft:max_health"` for any other attribute.

## `std.color`

Colors are `0xRRGGBB` ints, the form text components, particles and display entities use.

```mcfc
import std.color;

void main() {
    var orange = color.rgb(255, 128, 0);
    var hex = color.toHex(orange);
    var back = color.fromHex("#FF8000");
}
```

| Function | Returns |
| --- | --- |
| `int rgb(int red, int green, int blue)` | The color. Each channel is clamped to 0–255. |
| `int red(int color)`, `green`, `blue` | One channel, 0–255. |
| `String toHex(int color)` | `"#rrggbb"`, lowercase, for a text component's `color`. |
| `int fromHex(String hex)` | Reads `"#rrggbb"` or `"rrggbb"` in either case, or returns `-1` if `hex` isn't a hex color. |

## `std.dialog`

Shows a vanilla dialog to one player. Each button runs `/trigger <command>`, so it reaches the [`@Command`](./statements#command) handler of that name and needs no operator permissions.

```mcfc
import std.dialog;

@Command("buy")
void buy(Player player) {
    player.sendMessage("Bought a sword");
}

void openShop(Player player) {
    dialog.menu(player, "Shop", "Pick something", List.of(new dialog.Button("Buy sword", "buy")));
}
```

| Function | Shows |
| --- | --- |
| `void notice(Player player, String title, String body)` | A message with an OK button. |
| `void menu(Player player, String title, String body, List<Button> buttons)` | A message with one button per `Button(label, command)`. Needs at least one button. |

Dialogs are sent inline with `/dialog show`, so they need no registry entries and work after a `/reload`.

## Under The Hood

`std` is ordinary MCFC source compiled into the `mcfc` binary. Its functions lower like any other module function, for example `generated/std__math__clamp__d0__entry`. Unlike your own zero-argument `void` functions, `std` functions never get public `/function` wrappers.

`pow` runs a `while` loop, so its cost grows with `exponent`.
