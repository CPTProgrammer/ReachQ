//! Link reference resolution, owned by MarkdownSession.
//!
//! The lezer tree stays purely syntactic. This module watches
//! Link/Image/LinkReference nodes through the session's reconcile events,
//! maintains a document-ordered definition table (first definition wins),
//! and materializes resolved destinations directly onto Link/Image nodes as
//! synthetic URL/LinkTitle children — the render layer consumes them through
//! the same helpers it uses for inline links, unchanged.
//!
//! Incremental invariants:
//!
//! - Definition positions are absolute document offsets captured at event
//!   time and refreshed every update via `ChangeSet.mapPos`. mapPos is
//!   monotonic, so the registry stays sorted without re-sorting.
//! - Resolution state is a pure function of (link node, definition table).
//!   Synthetic children are dropped whenever reconcile reassigns a link's
//!   children, which produces a modify event that triggers re-resolution.
//! - A mapPos of null means the definition's start was swallowed by an edit;
//!   that block necessarily reparsed, so a remove/modify event follows and
//!   the entry is re-registered with a fresh position.

import { caseFold } from "unicode-case-folding";
import type { ChangeSet } from "./utils/change";
import { textChildren } from "./render";
import { setRefLabelNormalizer } from "./lezer/wrapper";
import type { Node } from "./markdown";

export interface NodeEvent {
	type: "add" | "remove" | "modify";
	node: Node<any>;
	/** Absolute document offset at event time (-1 for removals). */
	abs: number;
}

export const WATCHED_NODE_NAMES: ReadonlySet<string> = new Set(["Link", "Image", "LinkReference"]);

const MAX_LABEL_LENGTH = 999;

/**
 * Label matching per CommonMark: full Unicode case fold (via
 * unicode-case-folding, covering multi-char folds like ß → ss, ſ → s, ﬀ → ff),
 * then strip and collapse whitespace. Backslash escapes and entities are
 * deliberately NOT resolved — matching compares the raw source strings (spec:
 * "No other processing ... is done for the purposes of matching"), so
 * `[foo\!]` does not match `[foo!]:`.
 *
 * This is the single source of truth for label normalization: it is injected
 * into the parser's reference gate (see the bottom of this module), so the
 * session table and the parse-time gate always agree.
 */
export function normalizeLabel(raw: string): string {
	return caseFold(raw).replace(/\s+/g, " ").trim();
}

/** Source text of a node, reconstructed by concatenating leaf contents. */
function rawTextOf(node: Node<any>): string {
	if (node.content !== undefined) return node.content;
	let out = "";
	for (const child of node.children) out += rawTextOf(child);
	return out;
}

/** Inline `(...)` destination — such links never consult the definition table. */
function isInlineDest(node: Node<any>): boolean {
	return node.children.some((c) => c.name === "LinkMark" && c.content === "(");
}

/**
 * The reference label of a Link/Image: the explicit `[label]`, else the link
 * text. For collapsed/shortcut forms the label is the link text's RAW SOURCE
 * (emphasis/code marks are literal label characters: `[*foo* bar][]` matches
 * `[*foo* bar]:`), not its rendered plain text.
 */
function labelTextOf(node: Node<any>): string | null {
	for (const child of node.children) {
		if (child.name === "LinkLabel" && child.content !== undefined) {
			const inner = child.content.slice(1, -1);
			if (inner.trim() !== "") return inner.length > MAX_LABEL_LENGTH ? null : inner;
			break;
		}
	}
	let text = "";
	for (const child of textChildren(node)) text += rawTextOf(child);
	return text === "" || text.length > MAX_LABEL_LENGTH ? null : text;
}

interface DefEntry {
	node: Node<any>;
	abs: number;
	label: string;
	url: string;
	title: string | null;
}

/** Raw definition parts of a LinkReference node (URL/title keep their source form). */
function extractDef(node: Node<any>): { label: string; url: string; title: string | null } | null {
	let label: string | null = null, url: string | null = null, title: string | null = null;
	for (const child of node.children) {
		if (child.content === undefined) continue;
		if (child.name === "LinkLabel") {
			const inner = child.content.slice(1, -1);
			if (inner.length <= MAX_LABEL_LENGTH) label = normalizeLabel(inner);
		} else if (child.name === "URL") {
			url = child.content;
		} else if (child.name === "LinkTitle") {
			title = child.content;
		}
	}
	return label !== null && label !== "" && url !== null ? { label, url, title } : null;
}

function setEquals<T>(a: ReadonlySet<T>, b: ReadonlySet<T>): boolean {
	if (a.size !== b.size) return false;
	for (const v of a) if (!b.has(v)) return false;
	return true;
}

export class ReferenceResolver {
	/** Definitions sorted by absolute position (insertion order = document order). */
	private ordered: DefEntry[] = [];
	private entryOf = new Map<Node<any>, DefEntry>();
	/** First-wins definition table, rebuilt from `ordered` when definitions change. */
	private defs = new Map<string, { url: string; title: string | null }>();
	private linksByLabel = new Map<string, Set<Node<any>>>();
	private labelOfLink = new Map<Node<any>, string>();
	private defsDirty = false;
	private dirtyLinks = new Set<Node<any>>();
	private syntheticId = -1;

	/**
	 * Normalized label set for the parser's reference gate: which labels have
	 * a definition at all. The session feeds it to the parser before parsing
	 * and locally reparses bracket-containing inline containers when it
	 * changes (the tree depends on label membership, never on def values).
	 */
	gateLabels: ReadonlySet<string> = new Set();

	/**
	 * Applies one update's reconcile events. Returns whether the gate label
	 * set changed (the session then re-feeds the parser and reparses).
	 */
	flush(events: NodeEvent[], changeSet: ChangeSet): boolean {
		for (const entry of this.ordered) {
			const mapped = changeSet.mapPos(entry.abs);
			if (mapped !== null) entry.abs = mapped;
		}
		for (const event of events) this.onEvent(event);

		let labelsChanged = false;
		if (this.defsDirty) {
			this.defsDirty = false;
			const next = new Map<string, { url: string; title: string | null }>();
			for (const entry of this.ordered) {
				if (!next.has(entry.label)) next.set(entry.label, { url: entry.url, title: entry.title });
			}
			const changedLabels: string[] = [];
			for (const [label, def] of next) {
				const old = this.defs.get(label);
				if (!old || old.url !== def.url || old.title !== def.title) changedLabels.push(label);
			}
			for (const label of this.defs.keys()) if (!next.has(label)) changedLabels.push(label);
			this.defs = next;
			for (const label of changedLabels) {
				for (const link of this.linksByLabel.get(label) ?? []) this.dirtyLinks.add(link);
			}

			const keys = new Set(next.keys());
			labelsChanged = !setEquals(keys, this.gateLabels);
			if (labelsChanged) this.gateLabels = keys;
		}

		for (const link of this.dirtyLinks) this.resolve(link);
		this.dirtyLinks.clear();
		return labelsChanged;
	}

	private onEvent(event: NodeEvent) {
		const { node } = event;
		if (node.name === "LinkReference") {
			if (event.type !== "add") this.removeDef(node);
			if (event.type !== "remove") this.addDef(node, event.abs);
		} else {
			if (event.type !== "add") this.unregisterLink(node);
			if (event.type !== "remove") this.registerLink(node);
		}
	}

	private addDef(node: Node<any>, abs: number) {
		const def = extractDef(node);
		if (!def) return;
		const entry: DefEntry = { node, abs, ...def };
		let lo = 0, hi = this.ordered.length;
		while (lo < hi) {
			const mid = (lo + hi) >> 1;
			if (this.ordered[mid].abs < abs) lo = mid + 1; else hi = mid;
		}
		this.ordered.splice(lo, 0, entry);
		this.entryOf.set(node, entry);
		this.defsDirty = true;
	}

	private removeDef(node: Node<any>) {
		const entry = this.entryOf.get(node);
		if (!entry) return;
		this.entryOf.delete(node);
		const index = this.ordered.indexOf(entry);
		if (index >= 0) this.ordered.splice(index, 1);
		this.defsDirty = true;
	}

	private registerLink(node: Node<any>) {
		if (isInlineDest(node)) return;
		const raw = labelTextOf(node);
		if (raw === null) return;
		const label = normalizeLabel(raw);
		if (label === "") return;
		let set = this.linksByLabel.get(label);
		if (!set) this.linksByLabel.set(label, (set = new Set()));
		set.add(node);
		this.labelOfLink.set(node, label);
		this.dirtyLinks.add(node);
	}

	private unregisterLink(node: Node<any>) {
		const label = this.labelOfLink.get(node);
		if (label === undefined) return;
		this.labelOfLink.delete(node);
		const set = this.linksByLabel.get(label);
		if (set) {
			set.delete(node);
			if (set.size === 0) this.linksByLabel.delete(label);
		}
	}

	/** Materializes (or strips) the resolved destination as synthetic children. */
	private resolve(node: Node<any>) {
		const label = this.labelOfLink.get(node);
		const def = label !== undefined ? this.defs.get(label) : undefined;
		const children = node.children;
		let end = children.length;
		while (end > 0 && children[end - 1].synthetic) end--;
		if (!def) {
			if (end < children.length) node.children = children.slice(0, end);
			return;
		}
		const injected = children.slice(end);
		const curUrl = injected.find((c) => c.name === "URL");
		const curTitle = injected.find((c) => c.name === "LinkTitle");
		// Equality guard: don't churn the children array when nothing changed.
		if (curUrl?.content === def.url &&
			(def.title === null ? !curTitle : curTitle?.content === def.title)) return;
		const next: Node<any>[] = [this.synthetic("URL", def.url)];
		if (def.title !== null) next.push(this.synthetic("LinkTitle", def.title));
		node.children = [...children.slice(0, end), ...next];
	}

	private synthetic(name: string, content: string): Node<any> {
		return {
			id: this.syntheticId--,
			name,
			content,
			from: -1,
			to: -1,
			rawTree: null,
			children: [],
			synthetic: true,
		};
	}
}

// Single source of truth: the parser's reference gate normalizes candidate
// labels with this exact function (the patch falls back to its own pragmatic
// fold only when nothing was injected).
setRefLabelNormalizer(normalizeLabel);
