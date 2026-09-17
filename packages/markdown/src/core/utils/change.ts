import type { ChangedRange } from "@lezer/common";

export interface ChangeSet {
	mapPos(pos: number): number | null;
	intersectsOld(from: number, to: number): boolean;
	intersectsNew(from: number, to: number): boolean;
}

export const createChangeSet = (changes: ChangedRange[]): ChangeSet => (
	changes.length === 1 ? new SingleChangeSet(changes[0]) : new MultiChangeSet(changes)
);

class SingleChangeSet implements ChangeSet {
	private readonly delta: number;

	constructor(private readonly change: ChangedRange) {
		this.delta = (change.toB - change.fromB) - (change.toA - change.fromA);
	}

	mapPos(pos: number): number | null {
		const { fromA, toA } = this.change;
		return pos >= toA ? pos + this.delta : pos > fromA ? null : pos;
	}

	intersectsOld(from: number, to: number): boolean { return this.change.fromA < to && this.change.toA > from; }
	intersectsNew(from: number, to: number): boolean { return this.change.fromB < to && this.change.toB > from; }
}

class MultiChangeSet implements ChangeSet {
	private readonly deltas: number[] = [0];

	constructor(private readonly changes: ChangedRange[]) {
		for (const c of changes) {
			this.deltas.push(this.deltas.at(-1)! + (c.toB - c.fromB) - (c.toA - c.fromA));
		}
	}

	private upper(key: "toA" | "toB", pos: number): number {
		let lo = 0, hi = this.changes.length;
		while (lo < hi) {
			const mid = (lo + hi) >> 1;
			if (this.changes[mid][key] <= pos) {
				lo = mid + 1;
			} else {
				hi = mid;
			}
		}
		return lo;
	}

	mapPos(pos: number) {
		const i = this.upper("toA", pos);
		const change = this.changes[i];
		return change && change.fromA < pos && pos < change.toA ? null : pos + this.deltas[i];
	}

	intersectsOld(from: number, to: number): boolean {
		const change = this.changes[this.upper("toA", from)];
		return change !== undefined && change.fromA < to;
	}

	intersectsNew(from: number, to: number): boolean {
		const change = this.changes[this.upper("toB", from)];
		return change !== undefined && change.fromB < to;
	}
}
