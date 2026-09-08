/** Preset markdown source for the playground pages. */
export const SAMPLE = `# Streaming Markdown

The parent grows the **text prop** one chunk at a time, and the session
reparses *incrementally* via lezer TreeFragments — appending costs
\`O(delta)\` instead of \`O(document)\`.

## Features

- Incremental parsing with stable block identity
- Sealed blocks skip re-rendering (Svelte compares props by \`===\`)
- **Bold**, *italic*, ~~strikethrough~~, \`inline code\`

### Ordered list

1. Parse the new text with reuse fragments
2. Collect top-level blocks
3. Render only what changed

> Unchanged subtrees keep object identity in the new tree,
> which lets the component layer skip re-rendering them.

\`\`\`ts
const session = new MarkdownSession();
session.append('# Hello');
const blocks = session.blocks();
\`\`\`

---

| Feature | Status |
| ------- | ------ |
| Tables  | fallback (raw text) |

Done.
`;
