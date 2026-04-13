- state manipulation methods to arrays, push, pop, remove(idx)
- text_def that creates json text components, supports the whole spec at https://minecraft.wiki/w/Text_component_format

OVERHAUL:
- make MCFC modular and extendable. Developers should be able to create rust extensions to the compiler that adds more features like more commands, types, datastructures and more. Modularize the current compiler.