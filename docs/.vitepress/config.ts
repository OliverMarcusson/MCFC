import { defineConfig } from 'vitepress'
import mcfcGrammar from '../../editors/vscode-mcfc/syntaxes/mcfc.tmLanguage.json'

export default defineConfig({
  title: 'MCFC',
  description: 'A statically typed language, compiler, and language server for Minecraft datapacks.',
  cleanUrls: true,
  ignoreDeadLinks: false,
  markdown: {
    languages: [
      {
        ...(mcfcGrammar as any),
        aliases: ['mcfc', 'mcf']
      }
    ]
  },
  themeConfig: {
    logo: '/MCFC-icon.png',
    search: {
      provider: 'local'
    },
    nav: [
      { text: 'Guide', link: '/guide/introduction' },
      { text: 'Language', link: '/language/overview' },
      { text: 'Runtime', link: '/runtime/host-bridge' },
      { text: 'Examples', link: '/examples/' },
      { text: 'Editor', link: '/editor/vscode' },
      { text: 'Development', link: '/development/architecture' }
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/guide/introduction' },
          { text: 'Getting Started', link: '/guide/getting-started' },
          { text: 'CLI', link: '/guide/cli' },
          { text: 'Projects', link: '/guide/projects' }
        ]
      },
      {
        text: 'Language',
        items: [
          { text: 'Overview', link: '/language/overview' },
          { text: 'Syntax', link: '/language/syntax' },
          { text: 'Types and Builtins', link: '/language/types-and-builtins' },
          { text: 'Async and Runtime', link: '/language/async-and-runtime' },
          { text: 'Bukkit-style API', link: '/language/bukkit-style-api' },
          { text: 'Agent Events', link: '/language/agent-events' },
          { text: 'Limitations', link: '/language/limitations' }
        ]
      },
      {
        text: 'Language Reference',
        items: [
          { text: 'Statements', link: '/language/reference/statements' },
          {
            text: 'Statement Pages',
            items: [
              { text: 'fn', link: '/language/reference/statements/fn' },
              { text: 'data', link: '/language/reference/statements/data' },
              { text: 'event', link: '/language/reference/statements/event' },
              { text: 'command', link: '/language/reference/statements/command' },
              { text: 'task', link: '/language/reference/statements/task' },
              { text: 'player_state', link: '/language/reference/statements/player-state' },
              { text: 'struct', link: '/language/reference/statements/struct' },
              { text: 'let', link: '/language/reference/statements/let' },
              { text: 'assignment', link: '/language/reference/statements/assignment' },
              { text: 'if / else', link: '/language/reference/statements/if' },
              { text: 'match', link: '/language/reference/statements/match' },
              { text: 'while', link: '/language/reference/statements/while' },
              { text: 'for ranges', link: '/language/reference/statements/for-range' },
              { text: 'for selectors', link: '/language/reference/statements/for-selector' },
              { text: 'for arrays', link: '/language/reference/statements/for-array' },
              { text: 'async', link: '/language/reference/statements/async' },
              { text: 'break / continue / return', link: '/language/reference/statements/control-flow' },
              { text: 'mc', link: '/language/reference/statements/mc' },
              { text: 'mcf', link: '/language/reference/statements/mcf' },
              { text: 'as', link: '/language/reference/statements/as' },
              { text: 'at', link: '/language/reference/statements/at' },
              { text: 'function calls', link: '/language/reference/statements/call' }
            ]
          },
          { text: 'Raw Commands: mc', link: '/language/reference/raw-mc' },
          { text: 'Macro Commands: mcf', link: '/language/reference/macro-mcf' },
          { text: 'Built-in Types', link: '/language/reference/builtin-types' },
          {
            text: 'Type Pages',
            items: [
              { text: 'int', link: '/language/reference/types/int' },
              { text: 'bool', link: '/language/reference/types/bool' },
              { text: 'string', link: '/language/reference/types/string' },
              { text: 'array<T>', link: '/language/reference/types/array' },
              { text: 'dict<T>', link: '/language/reference/types/dict' },
              { text: 'entity_set', link: '/language/reference/types/entity-set' },
              { text: 'entity_ref', link: '/language/reference/types/entity-ref' },
              { text: 'player_ref', link: '/language/reference/types/player-ref' },
              { text: 'block_ref', link: '/language/reference/types/block-ref' },
              { text: 'entity_def', link: '/language/reference/types/entity-def' },
              { text: 'block_def', link: '/language/reference/types/block-def' },
              { text: 'item_def', link: '/language/reference/types/item-def' },
              { text: 'text_def', link: '/language/reference/types/text-def' },
              { text: 'item_slot', link: '/language/reference/types/item-slot' },
              { text: 'bossbar', link: '/language/reference/types/bossbar' },
              { text: 'nbt', link: '/language/reference/types/nbt' },
              { text: 'void', link: '/language/reference/types/void' },
              { text: 'struct types', link: '/language/reference/types/struct' }
            ]
          },
          { text: 'Function-style Builtins', link: '/language/reference/function-builtins' },
          {
            text: 'Builtin Pages',
            items: [
              { text: 'selector', link: '/language/reference/builtins/selector' },
              { text: 'single', link: '/language/reference/builtins/single' },
              { text: 'exists', link: '/language/reference/builtins/exists' },
              { text: 'has_data', link: '/language/reference/builtins/has-data' },
              { text: 'block', link: '/language/reference/builtins/block' },
              { text: 'entity', link: '/language/reference/builtins/entity' },
              { text: 'item', link: '/language/reference/builtins/item' },
              { text: 'text', link: '/language/reference/builtins/text' },
              { text: 'block_type', link: '/language/reference/builtins/block-type' },
              { text: 'bossbar', link: '/language/reference/builtins/bossbar' },
              { text: 'summon', link: '/language/reference/builtins/summon' },
              { text: 'debug', link: '/language/reference/builtins/debug' },
              { text: 'sleep', link: '/language/reference/builtins/sleep' },
              { text: 'sleep_ticks', link: '/language/reference/builtins/sleep-ticks' },
              { text: 'random', link: '/language/reference/builtins/random' },
              { text: 'int', link: '/language/reference/builtins/int' },
              { text: 'bool', link: '/language/reference/builtins/bool' },
              { text: 'string', link: '/language/reference/builtins/string' },
              { text: 'as', link: '/language/reference/builtins/as' },
              { text: 'at', link: '/language/reference/builtins/at' }
            ]
          },
          { text: 'Entity and Player Methods', link: '/language/reference/entity-player-methods' },
          {
            text: 'Method Pages',
            items: [
              { text: 'teleport', link: '/language/reference/methods/teleport' },
              { text: 'damage', link: '/language/reference/methods/damage' },
              { text: 'heal', link: '/language/reference/methods/heal' },
              { text: 'give', link: '/language/reference/methods/give' },
              { text: 'clear', link: '/language/reference/methods/clear' },
              { text: 'loot_give', link: '/language/reference/methods/loot-give' },
              { text: 'tellraw', link: '/language/reference/methods/tellraw' },
              { text: 'title', link: '/language/reference/methods/title' },
              { text: 'actionbar', link: '/language/reference/methods/actionbar' },
              { text: 'playsound', link: '/language/reference/methods/playsound' },
              { text: 'stopsound', link: '/language/reference/methods/stopsound' },
              { text: 'debug_entity', link: '/language/reference/methods/debug-entity' },
              { text: 'effect', link: '/language/reference/methods/effect' },
              { text: 'add_tag', link: '/language/reference/methods/add-tag' },
              { text: 'remove_tag', link: '/language/reference/methods/remove-tag' },
              { text: 'has_tag', link: '/language/reference/methods/has-tag' }
            ]
          },
          { text: 'Builders', link: '/language/reference/builders' },
          { text: 'Event Catalog', link: '/language/reference/event-catalog' },
          {
            text: 'Event Pages',
            items: [
              { text: 'player_join', link: '/language/reference/events/player-join' },
              { text: 'player_death', link: '/language/reference/events/player-death' },
              { text: 'chat', link: '/language/reference/events/chat' },
              { text: 'inventory_click', link: '/language/reference/events/inventory-click' },
              { text: 'player_action', link: '/language/reference/events/player-action' },
              { text: 'block_break', link: '/language/reference/events/block-break' },
              { text: 'player_interact_block', link: '/language/reference/events/player-interact-block' },
              { text: 'player_interact_item', link: '/language/reference/events/player-interact-item' },
              { text: 'entity_interact', link: '/language/reference/events/entity-interact' },
              { text: 'entity_attack', link: '/language/reference/events/entity-attack' },
              { text: 'item_held_change', link: '/language/reference/events/item-held-change' },
              { text: 'inventory_close', link: '/language/reference/events/inventory-close' },
              { text: 'player_swing', link: '/language/reference/events/player-swing' },
              { text: 'player_action_toggle', link: '/language/reference/events/player-action-toggle' },
              { text: 'player_respawn_request', link: '/language/reference/events/player-respawn-request' },
              { text: 'item_rename', link: '/language/reference/events/item-rename' },
              { text: 'trade_select', link: '/language/reference/events/trade-select' },
              { text: 'sign_change', link: '/language/reference/events/sign-change' },
              { text: 'book_edit', link: '/language/reference/events/book-edit' },
              { text: 'beacon_effect', link: '/language/reference/events/beacon-effect' },
              { text: 'recipe_place', link: '/language/reference/events/recipe-place' },
              { text: 'item_pick', link: '/language/reference/events/item-pick' },
              { text: 'entity_teleport', link: '/language/reference/events/entity-teleport' },
              { text: 'game_mode_request', link: '/language/reference/events/game-mode-request' },
              { text: 'player_abilities', link: '/language/reference/events/player-abilities' },
              { text: 'player_connect', link: '/language/reference/events/player-connect' },
              { text: 'player_quit', link: '/language/reference/events/player-quit' },
              { text: 'player_respawn', link: '/language/reference/events/player-respawn' },
              { text: 'player_damage', link: '/language/reference/events/player-damage' },
              { text: 'player_teleport', link: '/language/reference/events/player-teleport' },
              { text: 'player_item_drop', link: '/language/reference/events/player-item-drop' },
              { text: 'player_item_pickup', link: '/language/reference/events/player-item-pickup' },
              { text: 'inventory_open', link: '/language/reference/events/inventory-open' },
              { text: 'game_mode_change', link: '/language/reference/events/game-mode-change' }
            ]
          }
        ]
      },
      {
        text: 'Runtime',
        items: [
          { text: 'Host Bridge', link: '/runtime/host-bridge' },
          { text: 'Capabilities', link: '/runtime/capabilities' },
          { text: 'mcfd', link: '/runtime/mcfd' },
          { text: 'mcfd-agent', link: '/runtime/mcfd-agent' }
        ]
      },
      {
        text: 'Examples',
        items: [
          { text: 'Example Catalog', link: '/examples/' },
          { text: 'RPC Demo', link: '/examples/rpc-demo' },
          { text: 'Oracle', link: '/examples/oracle' },
          { text: 'Cyber Quotes', link: '/examples/cyber-quotes' },
          { text: 'Bukkit API Conformance', link: '/examples/bukkit-api-conformance' },
          { text: 'Debug Function Log', link: '/examples/debug-function-log' },
          { text: 'Generated Project', link: '/examples/generated-project' }
        ]
      },
      {
        text: 'Editor',
        items: [
          { text: 'VS Code', link: '/editor/vscode' }
        ]
      },
      {
        text: 'Development',
        items: [
          { text: 'Architecture', link: '/development/architecture' },
          { text: 'Contributing', link: '/development/contributing' }
        ]
      }
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/OliverMarcusson/MCFC' }
    ]
  }
})
