//! Incremental markdown parsing session built on @lezer/markdown.
//!
//! Lezer's TreeFragment lets the parser reuse unchanged subtrees across
//! reparses, so appending a streaming chunk costs O(delta) instead of
//! O(document). Unchanged subtrees keep object identity in the new tree,
//! which lets the component layer skip re-rendering them (Svelte props
//! compare by ===).

import { type ChangedRange, TreeFragment, type SyntaxNode, type Tree } from '@lezer/common';
import { GFM, type GetNodeNames, type TypedSyntaxNode, type TypedTree, parser as baseParser } from './lezer/wrapper';
import { TreeCursor } from '@lezer/common';
import { type ChangeSet, createChangeSet } from './utils/change';
import { summarizeRawBlock, type RawSummary } from './raw-html';
import { headingBlockNames, inlineContentBlockNames, rawTextBlockNames } from './lezer/node-types';

export const parser = baseParser.configure([GFM]);

const inlineSet: Set<string> = new Set(inlineContentBlockNames);
const headingSet: Set<string> = new Set(headingBlockNames);
const rawTextSet: Set<string> = new Set(rawTextBlockNames);

/** Reads a source range. Identity is stable per session, unlike `doc` itself. */
export type Slice = (from: number, to: number) => string;

/** A top-level block of the document (paragraph, heading, list, ...). */
export interface Node<N extends string> {
	id: number;

	name: N | "Text";

	/** Only available when itself is a leaf node (children.length = 0) */
	content?: string;

	/** Relative position of parent node */
	from: number;
	/** Relative position of parent node */
	to: number;

	rawTree: TypedTree<N> | null;
	children: Node<N>[];

	/**
	 * Raw HTML blocks only: source text plus tag-balance summary. Materialized
	 * by reconcile at the moment the block's content (re)assigns, so it is
	 * always in sync with `content`/`children` and grouping never rescans
	 * unchanged text.
	 */
	raw?: RawSummary | null;
}

// export interface NodeSplice<N extends string> {
// 	index: number;
// 	deleteCount: number;
// 	items: Node<N>[];
// 	removed: Node<N>[];
// }
// export interface NodeAmend {
// 	index: number;
// 	from?: number;
// 	to?: number;
// 	content?: string;
// }
// export interface NodeDiff<N extends string> {
// 	splices: NodeSplice<N>[];
// 	amends: NodeAmend[];
// 	children
// }

export interface Block<N extends string> {
	node: TypedSyntaxNode<N>;
	key: number;
	complete: boolean;
}

type Slot<N extends string> = {
	tree: TypedTree<N> | null;
	from: number; to: number;
};

function computeChangedRange(before: string, after: string): ChangedRange {
	let start = 0, endA = before.length, endB = after.length;
	const max = Math.min(endA, endB);
	while (start < max && before.charCodeAt(start) === after.charCodeAt(start)) start++;
	while (endA > start && endB > start && before.charCodeAt(endA - 1) === after.charCodeAt(endB - 1)) { endA--; endB--; }
	return { fromA: start, toA: endA, fromB: start, toB: endB };
}

// function intersectsChanges(changes: { fromA: number, toA: number }[], from: number, to: number): boolean {
// 	for (const change of changes) {
// 		if (change.fromA < to && change.toA > from) return true;
// 	}
// 	return false;
// }
// function intersectsChanges(changes: { fromB: number, toB: number }[], from: number, to: number): boolean {
// 	for (const change of changes) {
// 		if (change.fromB < to && change.toB > from) return true;
// 	}
// 	return false;
// }

// function textEqual(
// 	oldText: string, oldFrom: number,
// 	newText: string, newFrom: number,
// 	length: number
// ): boolean {
// 	for (let i = 0; i < length; i++)
// 		if (oldText.charCodeAt(oldFrom + i) !== newText.charCodeAt(newFrom + i)) return false;
// 	return true;
// }
// function deltaAt(changes: ChangedRange[], pos: number): number {
// 	let delta = 0;
// 	for (const c of changes) if (c.toA <= pos) delta += (c.toB - c.fromB) - (c.toA - c.fromA);
// 	return delta;
// }
// function sameBlock<N extends string>(
// 	prev: Node<N>,
// 	node: TypedSyntaxNode<N>,
// 	tree: TypedTree | null,
// 	changes: ChangedRange[],
// 	prevDoc: string,
// 	newDoc: string,
// ): boolean {
// 	if (prev.rawTree && prev.rawTree === tree) return true;
// 	if (prev.name !== node.name || prev.to - prev.from !== node.to - node.from) return false;
// 	if (intersectsChanges(changes, prev.from, prev.to) &&
// 		!textEqual(prevDoc, prev.from, newDoc, node.from, prev.to - prev.from)) return false;
// 	return prev.from === node.from + deltaAt(changes, node.from);
// }

function normalizeInlineNode<N extends string>(node: Node<N>) {
	if (!inlineSet.has(node.name)) return;
	const children = node.children;

	for (let i = 0; i < children.length; i++) {
		const child = children[i];
		if (child.name !== "Text" || !child.content) continue;
		const prev = children[i - 1];
		if (prev?.name === "QuoteMark") {
			// A blockquote marker consumes one optional following space.
			child.content = child.content.replace(/^ /, "");
		} else if (prev?.name === "HardBreak") {
			// Leading whitespace on the line after a hard break is stripped.
			child.content = child.content.replace(/^[ \t]+/, "");
		}
	}

	if (headingSet.has(node.name)) {
		// ATX: strip whitespace after the opening marker, and the whitespace
		// before a closing marker (`## foo ##` renders as `foo`).
		if (children[0]?.name === "HeaderMark") {
			const firstText = children.find(n => n.name === "Text");
			if (firstText?.content) firstText.content = firstText.content.replace(/^[ \t]+/, "");
			// Find the closing marker (a HeaderMark after the opening one).
			for (let i = children.length - 1; i >= 1; i--) {
				if (children[i].name === "HeaderMark") {
					const beforeMark = children[i - 1];
					if (beforeMark?.name === "Text" && beforeMark.content) {
						beforeMark.content = beforeMark.content.replace(/[ \t]+$/, "");
					}
					break;
				}
			}
		}
	}
	const lastChild = children.at(-1);
	if (lastChild?.name === "Text" && lastChild.content) {
		lastChild.content = lastChild.content.replace(/[ \t]+$/, "");
	}
}

function* childSlots<N extends string>(tree: TypedTree<N>, from: number, inline: boolean): Generator<Slot<N>> {
	let lastPos = from;
	const cursor = tree.cursor();
	for (let hasNode = cursor.firstChild(); hasNode; hasNode = cursor.nextSibling()) {
		if (cursor.tree === null) throw new Error("found TreeBuffer");
		const childFrom = from + cursor.from;
		const childTo = from + cursor.to;
		if (inline && childFrom > lastPos) yield {
			tree: null,
			from: lastPos,
			to: childFrom,
		};
		yield {
			tree: cursor.tree as TypedTree<N>,
			from: childFrom,
			to: childTo,
		};
		lastPos = childTo;
	}
	const to = from + tree.length;
	if (inline && lastPos < to) yield {
		tree: null,
		from: lastPos,
		to: to,
	};
}

function isLeafSlot<N extends string>(slot: Slot<N>): boolean {
	if (slot.tree === null) return true;
	return !inlineSet.has(slot.tree.type.name) && slot.tree.children.length === 0;
}

type DefaultParserNames = GetNodeNames<typeof parser>;
export class MarkdownSession {
	private doc = '';
	private tree: TypedTree<DefaultParserNames> | undefined;

	private nodeMap = new WeakMap<Tree, Node<DefaultParserNames>>();
	private nextId: number = 0;
	private nodes: Node<DefaultParserNames>[] = [];

	/** Stable identity across appends — components depend on this, not on `doc`. */
	readonly slice: Slice = (from, to) => this.doc.slice(from, to);

	/** Append a streaming chunk and reparse incrementally. */
	append(chunk: string) {
		const start = this.doc.length;
		return this.update(this.doc + chunk, [{ fromA: start, toA: start, fromB: start, toB: start + chunk.length }]);
	}

	update(input: string, changedRanges?: ChangedRange[]) {
		const changes = changedRanges ?? [computeChangedRange(this.doc, input)];
		let fragments = this.tree && TreeFragment.addTree(this.tree, []);
		if (fragments) fragments = TreeFragment.applyChanges(fragments, changes);
		this.doc = input;
		this.tree = parser.parse(this.doc, fragments);

		this.nodes = this.reconcileNodes(this.doc, this.tree, this.nodes, 0, 0, createChangeSet(changes));
		return this.nodes;
	}

	private reconcileNodes(
		newDoc: string, parentTree: TypedTree<DefaultParserNames>, nodes: Node<DefaultParserNames>[],
		newParentFrom: number, oldParentFrom: number, changeSet: ChangeSet,
		inline: boolean = false
	) {
		let nodeIndex = 0;
		const result: Node<DefaultParserNames>[] = [];

		const seek = (slot: Slot<DefaultParserNames>): Node<DefaultParserNames> | null => {
			const mapped = slot.tree ? this.nodeMap.get(slot.tree) : undefined;
			const name = slot.tree ? slot.tree.type.name : "Text";
			for (let node; (node = nodes[nodeIndex]); nodeIndex++) {
				const oldAbsFrom = oldParentFrom + node.from;
				const oldAbsTo = oldParentFrom + node.to;
				if (node === mapped || (
					node.name === name && isLeafSlot(slot) === (node.content !== undefined) &&
					changeSet.mapPos(oldAbsFrom) === slot.from &&
					changeSet.mapPos(oldAbsTo) === slot.to
				)) {
					nodeIndex++;
					return node;
				}

				const newAbsFrom = changeSet.mapPos(oldAbsFrom);
				if (!(newAbsFrom === null || newAbsFrom < slot.to || changeSet.intersectsOld(oldAbsFrom, oldAbsTo))) {
					return null;
				}
			}
			return null;
		}

		for (const slot of childSlots(parentTree, newParentFrom, inline)) {
			const node = seek(slot);
			if (!node) {
				result.push(this.buildSlot(slot, newDoc, newParentFrom, inline));
				continue;
			}

			const nodeOldFrom = node.from;
			const nodeOldTo = node.to;
			node.from = slot.from - newParentFrom;
			node.to = slot.to - newParentFrom;

			if (slot.tree ? node.rawTree !== slot.tree :
				changeSet.intersectsNew(slot.from, slot.to) ||
				changeSet.intersectsOld(oldParentFrom + nodeOldFrom, oldParentFrom + nodeOldTo)
			) {
				if (!slot.tree || isLeafSlot(slot)) {
					node.content = newDoc.slice(slot.from, slot.to);
				} else {
					node.rawTree = slot.tree;
					this.nodeMap.set(slot.tree, node);
					node.children = this.reconcileNodes(
						newDoc, slot.tree, node.children,
						slot.from, oldParentFrom + nodeOldFrom, changeSet,
						inline || inlineSet.has(node.name) || rawTextSet.has(node.name)
					);
					normalizeInlineNode(node);
				}
				node.raw = summarizeRawBlock(node);
			}
			result.push(node);
		}

		return result;
	}

	private buildSlot(slot: Slot<DefaultParserNames>, newDoc: string, parentFrom: number, parentInline: boolean): Node<DefaultParserNames> {
		if (!slot.tree) return {
			id: this.nextId++,
			name: "Text",
			content: newDoc.slice(slot.from, slot.to),
			from: slot.from - parentFrom,
			to: slot.to - parentFrom,
			rawTree: null,
			children: [],
		};

		const node: Node<DefaultParserNames> = {
			id: this.nextId++,
			name: slot.tree.type.name as DefaultParserNames,
			from: slot.from - parentFrom,
			to: slot.to - parentFrom,
			rawTree: slot.tree,
			children: [],
		};

		this.nodeMap.set(slot.tree, node);
		if (isLeafSlot(slot)) {
			node.content = newDoc.slice(slot.from, slot.to);
			node.raw = summarizeRawBlock(node);
			return node;
		}

		const inline = parentInline || inlineSet.has(node.name) || rawTextSet.has(node.name);
		for (const child of childSlots(slot.tree, slot.from, inline)) {
			node.children.push(this.buildSlot(child, newDoc, slot.from, inline));
		}

		normalizeInlineNode(node);
		node.raw = summarizeRawBlock(node);

		return node;
	}

	// private reconcileBlocks(oldDoc: string, newDoc: string, nodes: Node<N>[], cur: TreeCursor, changes: ChangedRange[], isInline?: boolean) {
	// 	const parentFrom = cur.from, parentTo = cur.to;
	// 	const splices: NodeSplice<N>[] = [];
	// 	const amends: NodeAmend<N>[] = [];

	// 	let nodeIndex = 0, ;
	// 	const newNodes: Node<N>[] =[];
	// 	for (let lastPos = parentFrom, childCur = cur.firstChild();
	// 		childCur;
	// 		lastPos = cur.to
	// 	) {
	// 		if (isInline && cur.from > lastPos) {
	// 			if (intersectsChanges(changes, lastPos, cur.from)) {
	// 				if (nodeIndex >= nodes.length || nodes[nodeIndex].name !== "Text") {
	// 					newNodes.push({
	// 						name: "Text",
	// 						content: newDoc.slice(cur.from, cur.to),
	// 						from: lastPos - parentFrom,
	// 						to: cur.from - parentFrom,
	// 						children: [],
	// 						rawTree: null,
	// 					});
	// 				} else {
	// 					const amend: NodeAmend<N> = {
	// 						index: nodeIndex,
	// 					};
	// 					let newFrom = lastPos - parentFrom; if (newFrom !== nodes[nodeIndex].from) {
	// 						nodes[nodeIndex].from = newFrom; amend.from = newFrom;
	// 					}
	// 					let newTo = cur.from - parentFrom; if (newTo !== nodes[nodeIndex].to) {
	// 						nodes[nodeIndex].to = newTo; amend.to = newTo;
	// 					}
	// 					if (amend.from !== undefined || amend.to !== undefined) {
	// 						let newText = newDoc.slice(lastPos, cur.from);
	// 						if (newText !== )
	// 					}
	// 				}
	// 				if (lastPos - parentFrom !== nodeIndex)
	// 			} else {
	// 				nodeIndex++;
	// 			}
	// 		}
	// 		if (cur.tree && this.nodeMap.get(cur.tree)) {
	// 			childCur = cur.nextSibling();
	// 			continue;
	// 			if (true) {
	// 				const removed: Node<N>[] = [];
	// 				let deleteCount = 0;
	// 				for (let i = Math.max(nodeIndex - 1, 0); i < nodeIndex; i++) {

	// 				}
	// 				splices.push({
	// 					index: nodeIndex,
	// 					deleteCount: deleteCount,
	// 					removed: removed,
	// 					items: newNodes,
	// 				});
	// 				nodeIndex++;
	// 			}
	// 		}
	// 		if (cur.name !== )
	// 	}
	// }

	get text(): string {
		return this.doc;
	}

	get syntaxTree(): TypedTree<DefaultParserNames> | undefined {
		return this.tree;
	}

	/** Top-level blocks; cheap to call after every append. */
	// blocks() {
	// 	const out: Block<GetNodeNames<typeof parser>>[] = [];
	// 	if (this.tree) {
	// 		for (let n = this.tree.topNode.firstChild; n; n = n.nextSibling) {
	// 			out.push({ key: out.length, node: n, complete: false });
	// 		}
	// 	}
	// 	// A block is sealed once a sibling follows it. The tail block is only
	// 	// sealed by a trailing blank line (a single \n may be a chunk boundary;
	// 	// the paragraph could still continue on the next line).
	// 	for (let i = 0; i < out.length - 1; i++) out[i].complete = true;
	// 	if (out.length && /\n[ \t]*\n[ \t]*$/.test(this.doc)) {
	// 		out[out.length - 1].complete = true;
	// 	}
	// 	return out;
	// }
}
