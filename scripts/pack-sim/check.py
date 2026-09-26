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
# improves; a rise is a regression.
BUDGET = {
    "control": 558,
    "data": 412,
    "sleepy": 169,
    "strings": 85,
    "switchy": 65,
    "timed": 388,
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
