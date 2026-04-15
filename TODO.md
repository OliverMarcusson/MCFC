<<<<<<< Updated upstream
TODOS:
- Add runtime read/writing to entity and block and item nbt data/components. No need for write to the player as it's not supported by minecraft.
=======
- text_def that creates json text components, supports the whole spec at https://minecraft.wiki/w/Text_component_format
>>>>>>> Stashed changes

OVERHAUL:
- make MCFC modular and extendable. Developers should be able to create rust extensions to the compiler that adds more features like more commands, types, datastructures and more. Modularize the current compiler.
