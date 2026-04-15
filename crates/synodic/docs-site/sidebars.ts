import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'intro',
    'getting-started',
    {
      type: 'category',
      label: 'CLI Reference',
      items: ['cli-reference'],
    },
    {
      type: 'category',
      label: 'API',
      items: ['api-reference', 'websocket'],
    },
    {
      type: 'category',
      label: 'Agent Integration',
      items: ['agent-integration', 'detection-rules'],
    },
    {
      type: 'category',
      label: 'Deployment',
      items: ['deployment'],
    },
  ],
};

export default sidebars;
