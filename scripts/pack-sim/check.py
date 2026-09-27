"""Check that optimization keeps behavior and does not get slower.

Builds every program in programs/ with and without optimization, runs both
in mcsim.py, and fails when either trace differs from <name>.expected or
the optimized build executes more commands than its budget below.

usage: python scripts/pack-sim/check.py
"""
import os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
PROGRAMS = os.path.join(HERE, "programs")

# Executed commands of the optimized build. Lower these when the optimizer
# improves; a rise is a regression. (`for` bounds are re-read every iteration,
# as in Java, which costs data/strings/timed a few commands over the old syntax.)
BUDGET = {
    "tests": 23,
    "recursion": 9313,
    "strescape": 341,
    "placeholders": 35,
    "actionbar": 323,
    "dialogs": 86,
    "blocktype": 256,
    "shapes": 9642,
    "noise": 3642,
    "colors": 3339,
    "strtools": 1525,
    "bits": 6987,
    "control": 554,
    "data": 421,
    "javaapi": 392,
    "javaish": 133,
    "sleepy": 172,
    "stdlib": 1317,
    "stdlib2": 1189,
    "strings": 120,
    "switchy": 69,
    "text": 13802,
    "timed": 393,
    "trig": 1427,
    "nested": 79,
    "objects": 339,
    "classes": 899,
    "inherit": 1179,
    "generics": 1293,
    "lambdas": 1392,
    "functions": 3008,
    "gc": 12272,
    "vectors": 380,
    "worldindex": 66,
}


def simulate(source, out, *flags):
    exe = os.path.join(ROOT, "target", "debug", "mcfc")
    subprocess.run([exe, "build", source, "--out", out, *flags], check=True, capture_output=True)
    result = subprocess.run(
        [sys.executable, os.path.join(HERE, "mcsim.py"), out],
        capture_output=True, text=True,
    )
    lines = result.stdout.splitlines()
    if result.returncode != 0 or not lines or not lines[-1].startswith("commands="):
        return None, result.stdout.strip()
    return int(lines[-1].split("=")[1]), "\n".join(lines[:-1])


def main():
    subprocess.run(["cargo", "build", "--quiet", "--bin", "mcfc"], cwd=ROOT, check=True)
    failed = False
    with tempfile.TemporaryDirectory() as tmp:
        for name in sorted(f[:-4] for f in os.listdir(PROGRAMS) if f.endswith(".mcf")):
            source = os.path.join(PROGRAMS, name + ".mcf")
            with open(os.path.join(PROGRAMS, name + ".expected"), encoding="utf8") as f:
                expected = f.read().strip()
            plain, plain_trace = simulate(source, os.path.join(tmp, name + "-plain"), "--no-optimize")
            opt, opt_trace = simulate(source, os.path.join(tmp, name + "-opt"))
            problems = []
            if plain_trace != expected:
                problems.append("unoptimized trace differs")
            if opt_trace != expected:
                problems.append("optimized trace differs")
            if opt is not None and opt > BUDGET.get(name, opt):
                problems.append(f"{opt} commands, budget {BUDGET[name]}")
            status = "; ".join(problems) or "ok"
            print(f"{name:10} plain {plain}  opt {opt}  {status}")
            if problems:
                failed = True
                print("  expected:\n    " + expected.replace("\n", "\n    "))
                print("  optimized:\n    " + opt_trace.replace("\n", "\n    "))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
