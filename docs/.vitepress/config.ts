import { defineConfig } from 'vitepress'

const repository = process.env.GITHUB_REPOSITORY?.split('/')[1]
const base =
  process.env.VITEPRESS_BASE ??
  (process.env.GITHUB_ACTIONS === 'true' && repository ? `/${repository}/` : '/')

export default defineConfig({
  base,
  title: 'Vantus',
  description:
    'Macro-first Rust backend framework with explicit composition, typed extraction, and hardened HTTP defaults.',
  cleanUrls: true,
  lastUpdated: true,
  srcExclude: ['**/node_modules/**', '**/.vitepress/**'],
  head: [
    ['meta', { name: 'theme-color', content: '#0f766e' }],
    ['meta', { property: 'og:title', content: 'Vantus Documentation' }],
    [
      'meta',
      {
        property: 'og:description',
        content:
          'Explicit Rust backend composition, typed extraction, and production-minded HTTP defaults.'
      }
    ]
  ],
  themeConfig: {
    logo: '/vantus-mark.svg',
    search: {
      provider: 'local'
    },
    nav: [
      { text: 'Quick Start', link: '/quick-start' },
      { text: 'API Reference', link: '/api-reference' },
      { text: 'Technical Deep Dive', link: '/technical-deep-dive' },
      { text: 'Production Notes', link: '/production-notes' }
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Overview', link: '/' },
          { text: 'Quick Start', link: '/quick-start' },
          { text: 'Advanced Demo', link: '/advanced-demo' }
        ]
      },
      {
        text: 'Reference',
        items: [
          { text: 'API Reference', link: '/api-reference' },
          { text: 'Configuration Reference', link: '/configuration-reference' },
          { text: 'Extraction Reference', link: '/extraction-reference' },
          { text: 'CLI Reference', link: '/cli-reference' }
        ]
      },
      {
        text: 'Architecture',
        items: [
          { text: 'Technical Deep Dive', link: '/technical-deep-dive' },
          { text: 'Migration to Macros', link: '/migration-to-macros' },
          { text: 'Production Notes', link: '/production-notes' }
        ]
      },
      {
        text: 'Operations',
        items: [{ text: 'Publishing Checklist', link: '/publishing-checklist' }]
      }
    ],
    outline: {
      level: [2, 3],
      label: 'On this page'
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/DiplomaCoder-cmyk/vantus' }
    ],
    editLink: {
      pattern: 'https://github.com/DiplomaCoder-cmyk/vantus/edit/main/docs/:path',
      text: 'Suggest an edit on GitHub'
    },
    footer: {
      message:
        'Built from the framework source to document the real request pipeline, routing model, and composition story.',
      copyright: 'Vantus documentation site'
    }
  }
})
