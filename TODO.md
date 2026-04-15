TODOS:
- Implement "mcfc new <project-name>" that initializes a mcfc project folder.
- Implement importing and exporting functions between .mcf files.
- Create a standard library std.mcf that exposes a rich library of common functions. Implement compiler not including functions that are not actually used in the output datapack.

OVERHAUL:
- make MCFC modular and extendable. Developers should be able to create rust extensions to the compiler that adds more features like more commands, types, datastructures and more. Modularize the current compiler.
