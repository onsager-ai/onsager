import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Synodic',
  tagline: 'The tool that watches the AI agents',
  favicon: 'img/favicon.ico',

  future: {
    v4: true,
  },

  url: 'https://codervisor.github.io',
  baseUrl: '/synodic/',

  organizationName: 'codervisor',
  projectName: 'synodic',

  onBrokenLinks: 'throw',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          editUrl: 'https://github.com/codervisor/synodic/tree/main/docs-site/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'Synodic',
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Docs',
        },
        {
          href: 'https://github.com/codervisor/synodic',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Documentation',
          items: [
            {label: 'Getting Started', to: '/docs/getting-started'},
            {label: 'CLI Reference', to: '/docs/cli-reference'},
            {label: 'API Reference', to: '/docs/api-reference'},
          ],
        },
        {
          title: 'Related',
          items: [
            {label: 'codervisor/eval', href: 'https://github.com/codervisor/eval'},
            {label: 'codervisor/orchestra', href: 'https://github.com/codervisor/orchestra'},
          ],
        },
        {
          title: 'More',
          items: [
            {label: 'GitHub', href: 'https://github.com/codervisor/synodic'},
          ],
        },
      ],
      copyright: `Copyright ${new Date().getFullYear()} Codervisor. Open source under MIT.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['bash', 'rust', 'json', 'yaml', 'toml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
