import { defineConfig } from 'vitepress'
import mcfcGrammar from '../../editors/vscode-mcfc/syntaxes/mcfc.tmLanguage.json'

export default defineConfig({
  title: 'MCFC',
  description: 'A statically typed language, compiler, and language server for Minecraft datapacks.',
  base: '/mcfc/',
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
      { text: 'Guide', link: '/guide/getting-started' },
      { text: 'Reference', link: '/language/reference/statements' },
      { text: 'Examples', link: '/examples/' }
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Getting Started', link: '/guide/getting-started' },
          { text: 'Your First Pack', link: '/guide/first-pack' },
          { text: 'Cookbook', link: '/guide/cookbook' },
          { text: 'Language Tour', link: '/language/tour' },
          { text: 'Projects', link: '/guide/projects' },
          { text: 'CLI', link: '/guide/cli' },
          { text: 'VS Code', link: '/editor/vscode' }
        ]
      },
      {
        text: 'Reference',
        items: [
          { text: 'Statements', link: '/language/reference/statements' },
          { text: 'Types', link: '/language/reference/types' },
          { text: 'Builtins', link: '/language/reference/builtins' },
          { text: 'Entities and Players', link: '/language/reference/methods' },
          { text: 'Builders', link: '/language/reference/builders' },
          { text: 'Events', link: '/language/reference/events' },
          { text: 'Standard Library', link: '/language/reference/std' },
          { text: 'How MCFC Compiles', link: '/language/reference/lowering' },
          { text: 'Limitations', link: '/language/limitations' }
        ]
      },
      {
        text: 'Host Bridge',
        items: [
          { text: 'Overview', link: '/runtime/host-bridge' },
          { text: 'Capabilities', link: '/runtime/capabilities' },
          { text: 'mcfd', link: '/runtime/mcfd' },
          { text: 'mcfd-agent', link: '/runtime/mcfd-agent' }
        ]
      },
      { text: 'Examples', link: '/examples/' },
      {
        text: 'Contributing',
        collapsed: true,
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
