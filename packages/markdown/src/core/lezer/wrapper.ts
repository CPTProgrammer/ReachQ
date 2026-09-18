import type { IterMode, NodeIterator, NodeType, PartialParse, SyntaxNode, SyntaxNodeRef, Tree } from "@lezer/common";
import { type DefaultNodeName, strikethroughNodeNames, tableNodeNames, taskListNodeNames } from "./node-types";
import { Autolink, type MarkdownConfig, type MarkdownExtension, type MarkdownParser, parser as rawParser, setLinkLabels, setRefLabelNormalizer, Strikethrough as rawStrikethrough, Table as rawTable, TaskList as rawTaskList } from "@lezer/markdown";

export type SpecName<S extends string | { readonly name: string }> =
	string extends S ? never : S extends string ? S :
	S extends { readonly name: infer N } ? (string extends N ? never : N) : never;

export type ExtensionNodeNames<E extends MarkdownExtension & { readonly nestedNames?: readonly string[] }> =
	| (E extends readonly MarkdownExtension[] ? ExtensionNodeNames<E[number]> : never)
	| (E extends { readonly nestedNames: infer Ns } ? Ns extends readonly unknown[] ? string extends Ns[number] ? never : Extract<Ns[number], string> : never : never)
	| (E extends { readonly defineNodes: infer Dn } ? Dn extends readonly (string | { readonly name: string })[] ? SpecName<Dn[number]> : never : never);

type WithTypedNodes<T, N extends string> =
	T extends SyntaxNode ? TypedSyntaxNode<N> :
	T extends SyntaxNodeRef ? TypedSyntaxNodeRef<N> :
	T extends Tree ? TypedTree<N> :
	T extends PartialParse ? TypedPartialParse<N> :
	T extends NodeIterator ? TypedNodeIterator<N> :
	T extends MarkdownParser ? TypedMarkdownParser<N> :
	T extends (...args: infer A) => infer R ?
		T extends (...args: any[]) => SyntaxNode | Tree ? (...args: A) => WithTypedNodes<R, N> : T :
	T extends readonly unknown[] ? { [I in keyof T]: WithTypedNodes<T[I], N> } : T;

export type TypedSyntaxNodeRef<N extends string = DefaultNodeName> = {
	[K in keyof SyntaxNodeRef]:
		K extends "name" ? N :
		K extends "type" ? NodeType & { readonly name: N } :
		WithTypedNodes<SyntaxNodeRef[K], N>;
};

export type TypedSyntaxNode<N extends string> = {
	[K in keyof SyntaxNode]:
		K extends "name" ? N :
		K extends "type" ? NodeType & { readonly name: N } :
		WithTypedNodes<SyntaxNode[K], N>;
};

export interface TypedIterateSpec<N extends string> {
	enter(node: TypedSyntaxNodeRef<N>): boolean | void;
	leave?(node: TypedSyntaxNodeRef<N>): void;
	from?: number;
	to?: number;
	mode?: IterMode;
}

export type TypedTree<N extends string = DefaultNodeName> = {
	[K in keyof Tree]:
		K extends "topNode" ? TypedSyntaxNode<N> :
		K extends "iterate" ? ((spec: TypedIterateSpec<N>) => void) & Tree[K] :
		WithTypedNodes<Tree[K], N>;
};

export type TypedPartialParse<N extends string = DefaultNodeName> = {
	[K in keyof PartialParse]: WithTypedNodes<PartialParse[K], N>;
};

export type TypedNodeIterator<N extends string = DefaultNodeName> = {
	[K in keyof NodeIterator]: WithTypedNodes<NodeIterator[K], N>;
};

export type TypedMarkdownParser<N extends string = DefaultNodeName> = {
	[K in keyof MarkdownParser]:
		K extends "configure" ? <const Ext extends MarkdownExtension>(spec: Ext) => TypedMarkdownParser<N | ExtensionNodeNames<Ext>> :
		WithTypedNodes<MarkdownParser[K], N>;
} & {
	readonly nodeTypes: { readonly [P in N]: number }
};

export type GetNodeNames<P extends { readonly nodeTypes: { readonly [K in string]: number } }> = keyof P["nodeTypes"];

export function withNestedNames<Ext extends MarkdownExtension, const Names extends readonly string[]>(
	extension: Ext,
	names: Names,
): Ext & { readonly nestedNames?: Names } {
	return extension as Ext & { readonly nestedNames?: Names };
}

export function asTypedParser<N extends string = DefaultNodeName>(p: MarkdownParser): TypedMarkdownParser<N> {
	return p as unknown as TypedMarkdownParser<N>;
}

export const parser = asTypedParser(rawParser);

/**
 * REACH PATCH hooks: register the reference label gate consulted by the
 * parser's `]` handler, and inject the label normalizer the gate uses.
 * Module-global in the patched parser, so the labels must be (re)set
 * immediately before every synchronous parse call.
 */
export { setLinkLabels, setRefLabelNormalizer };

export function withNodeNames<Ext extends MarkdownConfig, const Names extends readonly string[]>(
	extension: Ext,
	names: Names,
): Ext & { readonly defineNodes: Names } {
	return extension as Ext & { readonly defineNodes: Names };
}

export const Table = withNodeNames(rawTable, tableNodeNames);
export const TaskList = withNodeNames(rawTaskList, taskListNodeNames);
export const Strikethrough = withNodeNames(rawStrikethrough, strikethroughNodeNames);
export const GFM = [Table, TaskList, Strikethrough, Autolink] as const;
