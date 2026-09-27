"""Tiny mcfunction simulator for differential testing of MCFC output.

Supports the command subset MCFC emits for programs without entities or
floats. Anything else raises Unsupported, so a check can't silently pass.

usage: mcsim.py <datapack_dir> [ticks]  -> prints trace, then 'commands=N'
"""
import json, math, os, re, struct, sys


PROFILE = {} if os.environ.get("PROFILE") else None


class Unsupported(Exception):
    pass


class Return(Exception):
    def __init__(self, value, success=True):
        self.value, self.success = value, success


class Num:
    __slots__ = ("t", "v")

    def __init__(self, t, v):
        self.t, self.v = t, v

    def __eq__(self, o):
        return isinstance(o, Num) and o.t == self.t and o.v == self.v

    def __repr__(self):
        return self.snbt()

    def snbt(self):
        suffix = {"i": "", "b": "b", "s": "s", "l": "L", "f": "f", "d": "d"}[self.t]
        return f"{self.v}{suffix}"


def f32(v):
    return struct.unpack("f", struct.pack("f", v))[0]


def block_parts(block):
    """`id[a=b,c=d]` -> (id, {a: b, c: d})."""
    base, _, rest = block.partition("[")
    states = dict(pair.split("=") for pair in rest.rstrip("]").split(",") if pair)
    return base, states


def wrap32(v):
    return (v + 2**31) % 2**32 - 2**31


# ---------- SNBT ----------
class SnbtParser:
    def __init__(self, s):
        self.s, self.i = s, 0

    def ws(self):
        while self.i < len(self.s) and self.s[self.i] in " \t":
            self.i += 1

    def parse(self):
        self.ws()
        c = self.s[self.i]
        if c == "{":
            self.i += 1
            out = {}
            self.ws()
            if self.s[self.i] == "}":
                self.i += 1
                return out
            while True:
                self.ws()
                key = self.string_or_word()
                self.ws()
                assert self.s[self.i] == ":", self.s
                self.i += 1
                out[key] = self.parse()
                self.ws()
                if self.s[self.i] == ",":
                    self.i += 1
                    continue
                assert self.s[self.i] == "}", self.s
                self.i += 1
                return out
        if c == "[":
            self.i += 1
            if re.match(r"[BIL];", self.s[self.i:self.i + 2]):
                raise Unsupported("typed array")
            out = []
            self.ws()
            if self.s[self.i] == "]":
                self.i += 1
                return out
            while True:
                out.append(self.parse())
                self.ws()
                if self.s[self.i] == ",":
                    self.i += 1
                    continue
                assert self.s[self.i] == "]", self.s
                self.i += 1
                return out
        if c in "\"'":
            return self.quoted()
        word = self.word()
        return scalar(word)

    def quoted(self):
        q = self.s[self.i]
        self.i += 1
        out = []
        while self.s[self.i] != q:
            if self.s[self.i] == "\\":
                self.i += 1
                out.append({"n": "\n", "t": "\t"}.get(self.s[self.i], self.s[self.i]))
            else:
                out.append(self.s[self.i])
            self.i += 1
        self.i += 1
        return "".join(out)

    def word(self):
        start = self.i
        while self.i < len(self.s) and (self.s[self.i].isalnum() or self.s[self.i] in "_-.+"):
            self.i += 1
        return self.s[start:self.i]

    def string_or_word(self):
        return self.quoted() if self.s[self.i] in "\"'" else self.word()


def scalar(word):
    if word == "true":
        return Num("b", 1)
    if word == "false":
        return Num("b", 0)
    m = re.fullmatch(r"(-?\d+)([bBsSlL]?)", word)
    if m:
        return Num({"": "i", "b": "b", "s": "s", "l": "l"}[m.group(2).lower()], int(m.group(1)))
    m = re.fullmatch(r"(-?\d*\.?\d+(?:[eE]-?\d+)?)([fFdD]?)", word)
    if m:
        return Num(m.group(2).lower() or "d", float(m.group(1)))
    return word


def parse_snbt(s):
    p = SnbtParser(s)
    v = p.parse()
    p.ws()
    if p.i != len(s):
        raise Unsupported(f"trailing snbt {s!r}")
    return v


def quote_and_escape(s):
    """Minecraft's StringTag.quoteAndEscape: the first quote in `s` picks the other one."""
    out, quote = [], None
    for ch in s:
        if ch == "\\":
            out.append("\\")
        elif ch in "\"'":
            quote = quote or ("'" if ch == '"' else '"')
            if ch == quote:
                out.append("\\")
        elif ch in "\n\t\r\b\f":
            # ponytail: control-character escapes assumed from the 1.21.5 SNBT printer, unverified in 26.3.
            out.append("\\" + "ntrbf"["\n\t\r\b\f".index(ch)])
            continue
        out.append(ch)
    quote = quote or '"'
    return quote + "".join(out) + quote


def to_snbt(v):
    if isinstance(v, Num):
        return v.snbt()
    if isinstance(v, str):
        return quote_and_escape(v)
    if isinstance(v, list):
        return "[" + ",".join(to_snbt(x) for x in v) + "]"
    return "{" + ",".join(f"{k}:{to_snbt(x)}" for k, x in v.items()) + "}"


def macro_text(v):
    if isinstance(v, str):
        return v
    if isinstance(v, Num):
        return v.snbt()
    return to_snbt(v)


# ---------- NBT paths ----------
def parse_path(path):
    segs, i = [], 0
    while i < len(path):
        c = path[i]
        if c == ".":
            i += 1
            continue
        if c == "[":
            j = path.index("]", i)
            inner = path[i + 1:j]
            if inner == "":
                segs.append(("all",))
            elif re.fullmatch(r"-?\d+", inner):
                segs.append(("idx", int(inner)))
            else:
                raise Unsupported(f"path filter {path}")
            i = j + 1
            continue
        if c == "{":
            # A trailing compound filter, `a.b{k:v}`: shallow match only.
            segs.append(("filter", parse_snbt(path[i:])))
            break
        if c == '"':
            j = path.index('"', i + 1)
            segs.append(("key", path[i + 1:j]))
            i = j + 1
            continue
        j = i
        while j < len(path) and path[j] not in ".[{":
            j += 1
        segs.append(("key", path[i:j]))
        i = j
    return segs


def path_get(root, path):
    nodes = [root]
    for seg in parse_path(path):
        nxt = []
        for n in nodes:
            if seg[0] == "key" and isinstance(n, dict) and seg[1] in n:
                nxt.append(n[seg[1]])
            elif seg[0] == "idx" and isinstance(n, list):
                k = seg[1]
                if -len(n) <= k < len(n):
                    nxt.append(n[k])
            elif seg[0] == "all" and isinstance(n, list):
                nxt.extend(n)
            elif seg[0] == "filter" and isinstance(n, dict):
                if all(n.get(k) == v for k, v in seg[1].items()):
                    nxt.append(n)
        nodes = nxt
    return nodes


def path_parent(root, path, create):
    """(container, last_segment) for the single target of `path`."""
    segs = parse_path(path)
    node = root
    for seg in segs[:-1]:
        if seg[0] == "key":
            if seg[1] not in node:
                if not create:
                    return None, None
                node[seg[1]] = {}
            node = node[seg[1]]
        elif seg[0] == "idx":
            if not isinstance(node, list) or not -len(node) <= seg[1] < len(node):
                return None, None
            node = node[seg[1]]
        else:
            raise Unsupported("[] in write path")
    return node, segs[-1]


import copy


class Sim:
    def __init__(self, pack):
        self.funcs = {}
        self.tags = {}
        self.block_tags = {}
        # Placed blocks by (x, y, z); everything else is air.
        self.world = {}
        for r, _, names in os.walk(pack):
            for name in names:
                full = os.path.join(r, name)
                rel = os.path.relpath(full, pack).replace("\\", "/")
                m = re.fullmatch(r"data/([^/]+)/function/(.+)\.mcfunction", rel)
                if m:
                    lines = [l.strip() for l in open(full, encoding="utf8").read().splitlines()]
                    self.funcs[f"{m.group(1)}:{m.group(2)}"] = [l for l in lines if l and not l.startswith("#")]
                m = re.fullmatch(r"data/([^/]+)/tags/function/(.+)\.json", rel)
                if m:
                    self.tags[f"{m.group(1)}:{m.group(2)}"] = json.load(open(full))["values"]
                m = re.fullmatch(r"data/([^/]+)/tags/block/(.+)\.json", rel)
                if m:
                    self.block_tags[f"{m.group(1)}:{m.group(2)}"] = set(json.load(open(full))["values"])
        self.scores = {}
        self.storage = {}
        self.trace = []
        self.commands = 0
        self.gametime = 0
        self.schedule = {}
        self.limit = 20_000_000
        # Markers only: {"tags": set, "pos": [x, y, z]}. Position context of
        # `execute positioned`, absolute coordinates only.
        self.markers = []
        self.pos = [0.0, 0.0, 0.0]

    # --- functions ---
    def tag_functions(self, tag):
        """Function ids in a tag, expanding nested `#tags` and skipping missing optional ones."""
        for value in self.tags.get(tag, []):
            fid = value["id"] if isinstance(value, dict) else value
            if fid.startswith("#"):
                yield from self.tag_functions(fid[1:])
            else:
                yield fid

    def run_function(self, fid, args=None):
        if fid not in self.funcs:
            raise Unsupported(f"missing function {fid}")
        try:
            for line in self.funcs[fid]:
                if line.startswith("$"):
                    if args is None:
                        raise Unsupported(f"macro line without args in {fid}")
                    line = re.sub(r"\$\(([A-Za-z0-9_]+)\)", lambda m: macro_text(args[m.group(1)]), line[1:])
                self.command(line)
        except Return as r:
            return r.value, r.success
        return None, True

    def command(self, line):
        self.commands += 1
        if PROFILE is not None:
            PROFILE[line] = PROFILE.get(line, 0) + 1
        if self.commands > self.limit:
            raise Unsupported("command limit")
        return self.run(tokens(line), line)

    # returns (success, result)
    def run(self, t, line):
        head = t[0]
        if head == "execute":
            return self.execute(t[1:], line)
        if head == "scoreboard":
            return self.scoreboard(t[1:])
        if head == "data":
            return self.data(t[1:])
        if head == "function":
            fid = t[1]
            args = None
            if len(t) == 3 and t[2].startswith("{"):
                args = parse_snbt(t[2])
            elif len(t) > 2:
                if t[2:4] != ["with", "storage"]:
                    raise Unsupported(line)
                found = path_get(self.stor(t[4]), t[5]) if len(t) > 5 else [self.stor(t[4])]
                if not found:
                    return False, 0
                args = found[0]
            value, success = self.run_function(fid, args)
            return success, value if value is not None else 0
        if head == "return":
            if t[1] == "fail":
                raise Return(0, False)
            if t[1] == "run":
                ok, v = self.run(t[2:], line)
                raise Return(v, ok)
            raise Return(int(t[1]))
        if head == "title" and t[2] == "actionbar":
            self.trace.append(f"actionbar {t[1]} " + self.text(json.loads(" ".join(t[3:]))))
            return True, 1
        if head == "tellraw":
            self.trace.append("tellraw " + t[1] + " " + self.text(json.loads(" ".join(t[2:]))))
            return True, 1
        if head in ("say", "tell", "title", "dialog"):
            self.trace.append(" ".join(t))
            return True, 1
        if head == "schedule":
            if t[1] == "clear":
                self.schedule.pop(t[2], None)
                return True, 1
            delay = int(re.fullmatch(r"(\d+)t?", t[3]).group(1))
            mode = t[4] if len(t) > 4 else "replace"
            if mode == "replace":
                self.schedule[t[2]] = [self.gametime + delay]
            else:
                self.schedule.setdefault(t[2], []).append(self.gametime + delay)
            return True, 1
        if head == "compute" and t[1] == "default":
            result = self.compute(parse_snbt(" ".join(t[3:])))
            return True, wrap32(math.floor(result))
        if head == "summon" and t[1] == "minecraft:marker" and t[2:5] == ["~", "~", "~"]:
            nbt = parse_snbt(" ".join(t[5:])) if len(t) > 5 else {}
            self.markers.append({"tags": set(nbt.get("Tags", [])), "pos": list(self.pos)})
            return True, 1
        if head == "kill":
            before = len(self.markers)
            self.markers = [m for m in self.markers if not self.selects(t[1], m)]
            return before != len(self.markers), before - len(self.markers)
        if head == "setblock":
            self.world[tuple(int(c) for c in t[1:4])] = t[4]
            return True, 1
        if head == "clone" and t[1:4] == t[4:7]:
            source = tuple(int(c) for c in t[1:4])
            dest = tuple(int(c) for c in t[7:10])
            self.world[dest] = self.world.get(source, "minecraft:air")
            self.trace.append(" ".join(t))
            return True, 1
        if head == "fill":
            self.trace.append(" ".join(t))
            return True, 1
        if head == "time" and t[1:] == ["query", "gametime"]:
            return True, self.gametime
        raise Unsupported(line)

    def selects(self, selector, marker):
        m = re.fullmatch(r"@e\[(.*)\]", selector)
        if not m:
            raise Unsupported("selector " + selector)
        for arg in m.group(1).split(","):
            key, value = arg.split("=")
            if key == "tag" and value not in marker["tags"]:
                return False
            if key == "type" and value != "minecraft:marker":
                return False
        return True

    def text(self, c):
        """A text component as the plain text a player would read."""
        if isinstance(c, list):
            return "".join(self.text(x) for x in c)
        if isinstance(c, str):
            return c
        if "score" in c:
            return str(self.scores.get((c["score"]["name"], c["score"]["objective"]), 0))
        if "nbt" in c:
            found = path_get(self.stor(c["storage"]), c["nbt"])
            return "".join(v if isinstance(v, str) else to_snbt(v) for v in found)
        return c.get("text", "") + "".join(self.text(x) for x in c.get("extra", []))

    def stor(self, ns):
        return self.storage.setdefault(ns, {})

    def scoreboard(self, t):
        if t[0] == "objectives" or t[:2] == ["players", "enable"]:
            return True, 0
        op, holder = t[1], t[2]
        m = re.fullmatch(r"@a\[scores=\{([^=]+)=([^}]+)\}\]", holder)
        if m:
            # The pretend player is `@s`; skip when their score is out of range.
            have = self.scores.get(("@s", m.group(1)))
            if have is None or not in_range(have, m.group(2)):
                return True, 0
            holder = "@s"
        key = (holder, t[3]) if len(t) > 3 else None
        if op == "set":
            self.scores[key] = wrap32(int(t[4]))
            return True, self.scores[key]
        if op == "add":
            self.scores[key] = wrap32(self.scores.get(key, 0) + int(t[4]))
            return True, self.scores[key]
        if op == "remove":
            self.scores[key] = wrap32(self.scores.get(key, 0) - int(t[4]))
            return True, self.scores[key]
        if op == "get":
            if key not in self.scores:
                return False, 0
            return True, self.scores[key]
        if op == "reset":
            for k in [k for k in self.scores if k[0] == holder and (key is None or k == key)]:
                del self.scores[k]
            return True, 0
        if op == "operation":
            a, o, b = (t[2], t[3]), t[4], (t[5], t[6])
            x, y = self.scores.get(a, 0), self.scores.get(b, 0)
            if o == "=":
                x = y
            elif o == "+=":
                x = x + y
            elif o == "-=":
                x = x - y
            elif o == "*=":
                x = x * y
            elif o == "/=":
                if y != 0:
                    x = x // y
            elif o == "%=":
                if y != 0:
                    x = x - y * (x // y)
            elif o == "<":
                x = min(x, y)
            elif o == ">":
                x = max(x, y)
            elif o == "><":
                self.scores[b] = x
                x = y
            else:
                raise Unsupported(o)
            self.scores[a] = wrap32(x)
            return True, self.scores[a]
        raise Unsupported("scoreboard " + " ".join(t))

    def data(self, t):
        if t[0] == "modify" and t[1] == "storage":
            root, path, mode, rest = self.stor(t[2]), t[3], t[4], t[5:]
            index = None
            if mode == "insert":
                index, rest = int(rest[0]), rest[1:]
            kind = rest[0]
            if kind == "value":
                value = parse_snbt(" ".join(rest[1:]))
            elif kind == "from":
                if rest[1] != "storage":
                    raise Unsupported("data from " + rest[1])
                found = path_get(self.stor(rest[2]), rest[3]) if len(rest) > 3 else [self.stor(rest[2])]
                if not found:
                    return False, 0
                value = copy.deepcopy(found[0])
            elif kind == "string":
                if rest[1] != "storage":
                    raise Unsupported("string from " + rest[1])
                found = path_get(self.stor(rest[2]), rest[3])
                if not found:
                    return False, 0
                # A non-string source is read as its SNBT text.
                s = found[0] if isinstance(found[0], str) else to_snbt(found[0])
                start = int(rest[4]) if len(rest) > 4 else 0
                end = int(rest[5]) if len(rest) > 5 else len(s)
                value = s[start:end]
            elif kind == "compute":
                # `compute default float|integer <provider>`
                if rest[1] != "default":
                    raise Unsupported("compute at " + rest[1])
                result = self.compute(parse_snbt(" ".join(rest[3:])))
                value = Num("f", f32(result)) if rest[2] == "float" else Num("i", wrap32(math.floor(result)))
            else:
                raise Unsupported("data modify " + kind)
            return self.write(root, path, mode, value, index)
        if t[0] == "remove" and t[1] == "storage":
            parent, seg = path_parent(self.stor(t[2]), t[3], False)
            if parent is None:
                return False, 0
            if seg[0] == "key" and isinstance(parent, dict) and seg[1] in parent:
                del parent[seg[1]]
                return True, 1
            if seg[0] == "idx" and isinstance(parent, list) and -len(parent) <= seg[1] < len(parent):
                del parent[seg[1]]
                return True, 1
            return False, 0
        if t[0] == "get" and t[1] == "entity":
            found = [m for m in self.markers if self.selects(t[2], m)]
            axis = re.fullmatch(r"Pos\[(\d)\]", t[3])
            if not found or not axis:
                return False, 0
            scale = float(t[4]) if len(t) > 4 else 1
            return True, wrap32(math.floor(found[0]["pos"][int(axis.group(1))] * scale))
        if t[0] == "get" and t[1] == "storage":
            found = path_get(self.stor(t[2]), t[3]) if len(t) > 3 else [self.stor(t[2])]
            if not found:
                return False, 0
            v = found[0]
            scale = float(t[4]) if len(t) > 4 else 1
            if isinstance(v, Num):
                return True, wrap32(math.floor(v.v * scale))
            return True, len(v)
        raise Unsupported("data " + " ".join(t))

    def compute(self, p):
        """Evaluate a /compute number provider."""
        if isinstance(p, Num):
            return float(p.v)
        kind = p["type"].removeprefix("minecraft:")
        one = lambda: self.compute(p["input"])
        many = lambda: [self.compute(x) for x in p["inputs"]]
        if kind == "fixed" and "value" in p:
            return float(p["value"].v)
        if kind == "storage":
            found = path_get(self.stor(p["storage"]), p["path"])
            return float(found[0].v) if found and isinstance(found[0], Num) else 0.0
        if kind == "score":
            return float(self.scores.get((p["target"]["name"], p["score"]), 0))
        if kind in ("from_int", "float"):
            return one()
        if kind == "add":
            return sum(many())
        if kind == "mul":
            return math.prod(many())
        if kind == "min":
            return min(many())
        if kind == "max":
            return max(many())
        if kind == "length":
            return math.sqrt(sum(x * x for x in many()))
        if kind == "sub":
            return self.compute(p["left"]) - self.compute(p["right"])
        if kind == "div":
            right = self.compute(p["right"])
            return self.compute(p["left"]) / right if right else 0.0
        if kind == "pow":
            return math.pow(self.compute(p["base"]), self.compute(p["exponent"]))
        simple = {"negate": lambda x: -x, "floor": math.floor, "ceil": math.ceil,
                  "truncate": math.trunc, "abs": abs, "sqrt": math.sqrt,
                  "sin": math.sin, "cos": math.cos, "round": round}
        if kind in simple:
            return float(simple[kind](one()))
        raise Unsupported("compute " + kind)

    def write(self, root, path, mode, value, index):
        parent, seg = path_parent(root, path, True)
        if parent is None:
            return False, 0
        if mode == "set":
            if seg[0] == "key":
                if parent.get(seg[1]) == value:
                    return False, 0
                parent[seg[1]] = value
            elif seg[0] == "idx":
                if not isinstance(parent, list) or not -len(parent) <= seg[1] < len(parent):
                    return False, 0
                if parent[seg[1]] == value:
                    return False, 0
                parent[seg[1]] = value
            return True, 1
        if mode in ("append", "prepend", "insert"):
            if seg[0] == "key":
                lst = parent.setdefault(seg[1], [])
            else:
                lst = parent[seg[1]]
            if not isinstance(lst, list):
                return False, 0
            if mode == "append":
                lst.append(value)
            elif mode == "prepend":
                lst.insert(0, value)
            else:
                lst.insert(index, value)
            return True, 1
        if mode == "merge":
            target = parent.setdefault(seg[1], {})
            target.update(value)
            return True, 1
        raise Unsupported(mode)

    def execute(self, t, line):
        i = 0
        store = None
        while i < len(t):
            w = t[i]
            if w in ("if", "unless"):
                want = w == "if"
                kind = t[i + 1]
                if kind == "score":
                    a = (t[i + 2], t[i + 3])
                    if t[i + 4] == "matches":
                        ok = a in self.scores and in_range(self.scores[a], t[i + 5])
                        i += 6
                    else:
                        b = (t[i + 5], t[i + 6])
                        ok = a in self.scores and b in self.scores and compare(self.scores[a], t[i + 4], self.scores[b])
                        i += 7
                elif kind == "data" and t[i + 2] == "storage":
                    ok = bool(path_get(self.stor(t[i + 3]), t[i + 4]))
                    i += 5
                elif kind == "block" and t[i + 2:i + 5] == ["~", "~", "~"]:
                    here, here_states = block_parts(self.world.get(tuple(math.floor(c) for c in self.pos), "minecraft:air"))
                    test, states = block_parts(t[i + 5])
                    ok = here in self.block_tags[test[1:]] if test.startswith("#") else here == test
                    ok = ok and all(here_states.get(k) == v for k, v in states.items())
                    i += 6
                elif kind == "function":
                    value, success = self.run_function(t[i + 2])
                    ok = success and value not in (None, 0)
                    i += 3
                else:
                    raise Unsupported(line)
                if ok != want:
                    if store and "run" not in t[i:]:
                        # A failing final condition still stores its 0 result.
                        self.store(store, False, 0)
                    return False, 0
                continue
            if w == "store":
                what = t[i + 1]
                if t[i + 2] == "score":
                    store = (what, "score", (t[i + 3], t[i + 4]))
                    i += 5
                elif t[i + 2] == "storage":
                    store = (what, "storage", t[i + 3], t[i + 4], t[i + 5], float(t[i + 6]))
                    i += 7
                else:
                    raise Unsupported(line)
                continue
            if w == "positioned" and all(re.fullmatch(r"-?[\d.]+", c) for c in t[i + 1:i + 4]):
                self.pos = [float(c) for c in t[i + 1:i + 4]]
                i += 4
                continue
            if w == "as" and t[i + 1] in ("@a", "@s"):
                # One pretend player: enough for code that talks to players.
                i += 2
                continue
            if w == "align" and t[i + 1] == "xyz":
                self.pos = [float(math.floor(c)) for c in self.pos]
                i += 2
                continue
            if w == "run":
                ok, v = self.run(t[i + 1:], line)
                break
            raise Unsupported(line)
        else:
            ok, v = True, 1
        if store:
            self.store(store, ok, v)
        return ok, v

    def store(self, store, ok, v):
        if True:
            val = v if store[0] == "result" else int(ok)
            if not ok and store[0] == "result":
                val = 0
            if store[1] == "score":
                self.scores[store[2]] = wrap32(int(val))
            else:
                _, _, ns, path, typ, scale = store
                num = val * scale
                tc = {"int": "i", "byte": "b", "short": "s", "long": "l", "float": "f", "double": "d"}[typ]
                num = float(num) if tc in "fd" else math.floor(num)
                self.write(self.stor(ns), path, "set", Num(tc, num), None)

    # --- driver ---
    def load(self, ticks):
        for fid in self.tag_functions("minecraft:load"):
            self.run_function(fid)
        for _ in range(ticks):
            self.gametime += 1
            due = sorted(f for f, times in self.schedule.items() if any(x <= self.gametime for x in times))
            for fid in due:
                times = [x for x in self.schedule[fid] if x > self.gametime]
                if times:
                    self.schedule[fid] = times
                else:
                    del self.schedule[fid]
                self.run_function(fid)
            for fid in self.tag_functions("minecraft:tick"):
                self.run_function(fid)


def in_range(v, r):
    if ".." in r:
        lo, hi = r.split("..")
        return (lo == "" or v >= int(lo)) and (hi == "" or v <= int(hi))
    return v == int(r)


def compare(a, op, b):
    return {"<": a < b, "<=": a <= b, "=": a == b, ">": a > b, ">=": a >= b}[op]


def tokens(line):
    out, cur, depth, quote = [], [], 0, None
    for ch in line:
        if quote:
            cur.append(ch)
            if ch == quote and (len(cur) < 2 or cur[-2] != "\\"):
                quote = None
            continue
        if ch in "\"'":
            quote = ch
        elif ch in "{[":
            depth += 1
        elif ch in "}]":
            depth -= 1
        if ch == " " and depth == 0:
            if cur:
                out.append("".join(cur))
                cur = []
            continue
        cur.append(ch)
    if cur:
        out.append("".join(cur))
    return out


if __name__ == "__main__":
    sim = Sim(sys.argv[1])
    try:
        sim.load(int(sys.argv[2]) if len(sys.argv) > 2 else 40)
        for line in sim.trace:
            print(line)
        print(f"commands={sim.commands}")
        if PROFILE is not None:
            kinds = {}
            for line, n in PROFILE.items():
                k = re.sub(r"\$d\d+_\w+|frames\.\S+|-?\d+|\"[^\"]*\"|\S+:\S+", "_", line)[:70]
                kinds[k] = kinds.get(k, 0) + n
            for k, n in sorted(kinds.items(), key=lambda kv: -kv[1])[:25]:
                print(f"{n:6} {k}", file=sys.stderr)
            if os.environ.get("PROFILE") == "raw":
                for k, n in sorted(PROFILE.items(), key=lambda kv: -kv[1])[:40]:
                    print(f"{n:6} {k[:150]}", file=sys.stderr)
    except Unsupported as e:
        print(f"UNSUPPORTED: {e}")
        sys.exit(2)
