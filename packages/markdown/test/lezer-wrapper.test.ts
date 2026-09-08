import { GFM, parser, Strikethrough, Table, TaskList } from "@lezer/markdown";
import { describe, expect, it } from "vitest";
import { defaultNodeNames, strikethroughNodeNames, tableNodeNames, taskListNodeNames } from "../src/core/lezer/node-types";
import { GFM as typedGFM } from "../src/core/lezer/wrapper";

describe("Lezer Typed Wrapper", () => {
	it("defaultNodeNames", () => {
		expect(defaultNodeNames.toSorted())
			.toEqual(parser.nodeSet.types.map(type => type.name).filter(Boolean).toSorted());
	});
	it("tableNodeNames", () => {
		expect(tableNodeNames.toSorted())
			.toEqual(Table.defineNodes!.map(type => typeof type === "string" ? type : type.name).filter(Boolean).toSorted());
	});
	it("taskListNodeNames", () => {
		expect(taskListNodeNames.toSorted())
			.toEqual(TaskList.defineNodes!.map(type => typeof type === "string" ? type : type.name).filter(Boolean).toSorted());
	});
	it("strikethroughNodeNames", () => {
		expect(strikethroughNodeNames.toSorted())
			.toEqual(Strikethrough.defineNodes!.map(type => typeof type === "string" ? type : type.name).filter(Boolean).toSorted());
	});
	it("GFM", () => {
		expect(typedGFM).toEqual(GFM);
	});
});
