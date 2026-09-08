export const defaultNodeNames = [
	"Document",

	"CodeBlock",
	"FencedCode",
	"Blockquote",
	"HorizontalRule",
	"BulletList",
	"OrderedList",
	"ListItem",
	"ATXHeading1",
	"ATXHeading2",
	"ATXHeading3",
	"ATXHeading4",
	"ATXHeading5",
	"ATXHeading6",
	"SetextHeading1",
	"SetextHeading2",
	"HTMLBlock",
	"LinkReference",
	"Paragraph",
	"CommentBlock",
	"ProcessingInstructionBlock",

	// Inline
	"Escape",
	"Entity",
	"HardBreak",
	"Emphasis",
	"StrongEmphasis",
	"Link",
	"Image",
	"InlineCode",
	"HTMLTag",
	"Comment",
	"ProcessingInstruction",
	"Autolink",

	// Smaller tokens
	"HeaderMark",
	"QuoteMark",
	"ListMark",
	"LinkMark",
	"EmphasisMark",
	"CodeMark",
	"CodeText",
	"CodeInfo",
	"LinkTitle",
	"LinkLabel",
	"URL"
] as const;

export const tableNodeNames = [ "Table", "TableHeader", "TableRow", "TableCell", "TableDelimiter" ] as const;
export const taskListNodeNames = [ "Task", "TaskMarker" ] as const;
export const strikethroughNodeNames = ["Strikethrough", "StrikethroughMark"] as const;

export type DefaultNodeName = typeof defaultNodeNames[number];

export type TableNodeName = typeof tableNodeNames[number];
export type TaskListNodeName = typeof taskListNodeNames[number];
export type StrikethroughNodeName = typeof strikethroughNodeNames[number];

export type GFMNodeName = TableNodeName | TaskListNodeName | StrikethroughNodeName;
export type AllNodeName = DefaultNodeName | GFMNodeName;

export const inlineContentBlockNames = [
	"Paragraph", "TableCell",
	"ATXHeading1", "ATXHeading2", "ATXHeading3",
	"ATXHeading4", "ATXHeading5", "ATXHeading6",
	"SetextHeading1", "SetextHeading2"
] as const satisfies AllNodeName[];

export const headingBlockNames = [
	"ATXHeading1", "ATXHeading2", "ATXHeading3",
	"ATXHeading4", "ATXHeading5", "ATXHeading6",
	"SetextHeading1", "SetextHeading2"
] as const satisfies AllNodeName[];

export type InlineContentBlockName = typeof inlineContentBlockNames[number];
