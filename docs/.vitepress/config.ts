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
