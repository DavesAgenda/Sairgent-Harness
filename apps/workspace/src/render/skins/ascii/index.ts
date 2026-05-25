import type { WorkspaceSkin } from '../../skinTypes';
import { WorkspaceCanvas } from './WorkspaceCanvas';
import { InboxTray } from './InboxTray';

export { AgentDesk } from './AgentDesk';
export { BenchRow } from './BenchRow';
export { TubeOverlay } from './TubeOverlay';
export { TubeCapsule } from './TubeCapsule';
export { WorkspaceCanvas } from './WorkspaceCanvas';
export { InboxTray } from './InboxTray';

const asciiSkin: WorkspaceSkin = {
  id: 'ascii',
  name: 'ASCII Terminal',
  description: 'Green-on-black terminal aesthetic with box-drawing characters.',
  WorkspaceCanvas,
  InboxTray,
};

export default asciiSkin;
