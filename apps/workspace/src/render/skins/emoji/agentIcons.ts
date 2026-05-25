/**
 * Map agent names to emoji icons for the emoji skin.
 * Falls back to a generic icon for unknown agents.
 */
const EMOJI_ICONS: Record<string, string> = {
  perry:    '\uD83D\uDC68\u200D\uD83D\uDCBC', // man office worker
  felicity: '\uD83D\uDC69\u200D\uD83D\uDCBB', // woman technologist
  jimmy:    '\uD83C\uDFA8',                     // artist palette
  lois:     '\uD83D\uDD0D',                     // magnifying glass
  lex:      '\uD83D\uDCB0',                     // money bag
  cat:      '\uD83D\uDCE3',                     // megaphone
  clark:    '\uD83D\uDCDD',                     // memo
  ray:      '\u2699\uFE0F',                     // gear
  kryptonite: '\uD83D\uDD12',                   // lock
  oracle:   '\uD83D\uDD2E',                     // crystal ball
};

const FALLBACK = '\uD83E\uDD16'; // robot

export function emojiIcon(agentName: string): string {
  return EMOJI_ICONS[agentName.toLowerCase()] ?? FALLBACK;
}
