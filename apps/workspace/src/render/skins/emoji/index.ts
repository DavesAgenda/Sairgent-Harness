import type { WorkspaceSkin } from '../../skinTypes';
import { EmojiWorkspaceCanvas } from './EmojiWorkspaceCanvas';
import { EmojiInboxTray } from './EmojiInboxTray';

const emojiSkin: WorkspaceSkin = {
  id: 'emoji',
  name: 'Emoji Factory',
  description: 'Colourful emoji-driven skin with a lighter palette.',
  WorkspaceCanvas: EmojiWorkspaceCanvas,
  InboxTray: EmojiInboxTray,
};

export default emojiSkin;
