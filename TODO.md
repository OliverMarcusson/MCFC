TODOS:
- Implement "mcfc new <project-name>" that initializes a mcfc project folder.

OVERHAUL:
- make MCFC modular and extendable. Developers should be able to create rust extensions to the compiler that adds more features like more commands, types, datastructures and more. Modularize the current compiler.

BUGS (found while writing docs):
- Whole-program errors (recursion, `event.cancel()` on an observation-only event) point at line 1 and print a `# source: <path>` line instead of the offending code.
- An empty `[]` passed as a function argument fails with "empty array literals require type context" even when the parameter type is known.
- `mcfc new` creates `assets/.gitkeep`, and the build copies it into `dist/`.
