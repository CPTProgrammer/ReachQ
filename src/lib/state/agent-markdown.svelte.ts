//! Markdown sessions for assistant text blocks.
//!
//! `MarkdownSession` lives on the content block object itself (symbol props),
//! so its lifetime follows the block: a snapshot replaces `rt.messages`
//! wholesale and dropped blocks take their session along via GC — no registry
//! to sweep. Svelte's `$state` proxies only wrap plain objects/arrays, so the
//! class instance survives reads through the reactive graph intact, while the
//! nodes array is plain data that the component layer consumes by `node.id`.
//!
//! The session owns the block's text: `mdAppendDelta` is the ONLY writer of
//! an assistant text block's `text`, so `session.text` and `block.text` stay
//! in sync by construction (snapshots arrive as fresh block objects and are
//! seeded by `mdSyncSnapshot`).

import { MarkdownSession, parser, type GetNodeNames, type Node } from '@reach/markdown';
import type { ContentBlock, PathMessage } from '$lib/ipc/agent';

export type MdNode = Node<GetNodeNames<typeof parser>>;

const SESSION = Symbol('agent-md.session');
const NODES = Symbol('agent-md.nodes');

type TextBlock = Extract<ContentBlock, { type: 'text' }>;
type Decorated = TextBlock & { [SESSION]?: MarkdownSession; [NODES]?: MdNode[] };

/**
 * Parsed nodes for an assistant text block; undefined until the block's first
 * delta or snapshot sync. Template-safe read — tracks through the block proxy.
 */
export function getMdNodes(block: ContentBlock): MdNode[] | undefined {
	return block.type === 'text' ? (block as Decorated)[NODES] : undefined;
}

/** Streaming entry point: append one `text_delta` chunk to the block. */
export function mdAppendDelta(block: TextBlock, delta: string): void {
	const b = block as Decorated;
	let session = b[SESSION];
	if (!session) {
		session = new MarkdownSession();
		session.update(b.text);
		b[SESSION] = session;
	} else if (session.text.length !== b.text.length) {
		// Defensive: some path wrote block.text without going through this
		// module. Resync instead of appending onto a diverged document.
		console.warn('[agent-markdown] text block diverged from its session; resyncing', {
			sessionLength: session.text.length,
			blockLength: b.text.length
		});
		session.update(b.text);
	}
	b[NODES] = session.append(delta);
	b.text = session.text;
}

/**
 * Snapshot seeding: attach a session to every assistant text block. Blocks
 * from the real backend are fresh IPC objects; the mock backend shares block
 * objects across snapshots, so a block may already carry a session — reuse it
 * when the text hasn't moved.
 */
export function mdSyncSnapshot(messages: PathMessage[]): void {
	for (const msg of messages) {
		if (msg.role !== 'assistant') continue;
		for (const block of msg.content) {
			if (block.type !== 'text') continue;
			const b = block as Decorated;
			const existing = b[SESSION];
			if (existing && existing.text === b.text && b[NODES]) continue;
			const session = new MarkdownSession();
			b[SESSION] = session;
			b[NODES] = session.update(b.text);
		}
	}
}
